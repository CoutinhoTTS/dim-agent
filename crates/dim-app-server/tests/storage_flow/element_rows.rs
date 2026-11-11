use super::*;
use dim_agent_runtime::{
    QueryComparison, QueryFilter, QueryOperand, QueryOrder, QueryOrderKey, QueryOutput, QueryPage,
    QueryPick, QueryRows, QuerySelect, QuerySpec, QueryValue, TimelineQueryDirection,
    TimelineReadScope, TimelineReadSegment,
};
use serde_json::{Value, json};
use std::num::NonZeroU64;

fn eq(path: &str, value: Value) -> QueryFilter {
    QueryFilter::Compare {
        left: QueryOperand::Field { path: path.into() },
        comparison: QueryComparison::Eq,
        right: QueryOperand::Value { value },
    }
}

fn query(fields: &[&str]) -> QuerySpec {
    QuerySpec {
        rows: Some(QueryRows::Elements {
            field: "data.changes".into(),
        }),
        filter: Some(eq("kind", json!("rows/array"))),
        select: QuerySelect::Fields {
            fields: fields.iter().map(|field| (*field).into()).collect(),
        },
        group_by: vec![],
        pick: None,
        order_by: vec![],
    }
}

fn page() -> QueryPage {
    QueryPage {
        limit: NonZeroU64::new(2).unwrap(),
        after: None,
    }
}

#[tokio::test]
async fn paired_element_selectors_do_not_expand_an_unrelated_event_of_another_value_type() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let memory = MemoryTimelineStore::default();
    store
        .create_session("rows", &SessionConfig::default())
        .await
        .unwrap();
    memory
        .create_session("rows", &SessionConfig::default())
        .await
        .unwrap();
    let records = [
        json!({"changes":["first"]}),
        json!({"changes":["second"]}),
        json!({"changes":"unrelated scalar"}),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, data)| {
        record(
            u64::try_from(index).unwrap() + 1,
            EventKind::Extension(ExtensionEvent::new("rows", "array", data).unwrap()),
        )
    })
    .collect::<Vec<_>>();
    append(&store, key("rows", "main"), records.clone())
        .await
        .unwrap();
    memory
        .append(
            key("rows", "main"),
            records,
            Box::new(|result| result.unwrap()),
        )
        .unwrap();
    let scope = TimelineReadScope {
        key: key("rows", "main"),
        through: 3,
        segments: vec![TimelineReadSegment {
            branch_id: "main".into(),
            from: 1,
            through: 3,
            stored_through: 3,
            tail: vec![],
        }],
    };
    let mut q = query(&["id", "element.value"]);
    q.filter = Some(QueryFilter::And {
        conditions: vec![
            eq("kind", json!("rows/array")),
            QueryFilter::Or {
                conditions: vec![
                    QueryFilter::And {
                        conditions: vec![eq("id", json!("event-1")), eq("element.index", json!(0))],
                    },
                    QueryFilter::And {
                        conditions: vec![eq("id", json!("event-2")), eq("element.index", json!(0))],
                    },
                ],
            },
        ],
    });
    let expected = memory.query(&scope, &q, Some(&page())).await;
    let actual = store.query(&scope, &q, Some(&page())).await;
    assert!(
        expected.is_ok() && actual.is_ok(),
        "only the two named arrays are query inputs: memory={expected:?}, sqlite={actual:?}"
    );
    assert_eq!(
        serde_json::to_value(expected.unwrap()).unwrap(),
        serde_json::to_value(actual.unwrap()).unwrap()
    );
    q.filter = Some(QueryFilter::And {
        conditions: vec![
            eq("kind", json!("rows/array")),
            QueryFilter::Or {
                conditions: vec![eq("id", json!("event-1")), eq("element.index", json!(2))],
            },
        ],
    });
    assert!(matches!(
        memory.query(&scope, &q, Some(&page())).await,
        Err(AgentError::InvalidInput(_))
    ));
    assert!(matches!(
        store.query(&scope, &q, Some(&page())).await,
        Err(AgentError::InvalidInput(_))
    ));
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn v38_array_upgrade_is_atomic_reuses_versions_and_rejects_missing_locators() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    store
        .create_session("rows", &SessionConfig::default())
        .await
        .unwrap();
    let records = (1..=2)
        .map(|sequence| {
            record(
                sequence,
                EventKind::Extension(
                    ExtensionEvent::new("legacy", "array", json!({"changes":[]})).unwrap(),
                ),
            )
        })
        .collect();
    append(&store, key("rows", "main"), records).await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
    drop(store);
    drop(db);
    let values = (0..257)
        .map(|index| json!({"number":index,"body":"large old value ".repeat(16)}))
        .collect::<Vec<_>>();
    let connection = Connection::open(&database.0).unwrap();
    let mut originals = Vec::new();
    for sequence in 1..=2 {
        let body: String = connection
            .query_row(
                "SELECT body FROM timeline_events WHERE session_id='rows' AND sequence=?1",
                [sequence],
                |row| row.get(0),
            )
            .unwrap();
        let mut wire: Value = serde_json::from_str(&body).unwrap();
        wire["record"]["event"]["data"]["changes"] = json!(values);
        originals.push(wire["record"]["event"].clone());
        connection
            .execute(
                "UPDATE timeline_events SET body=?2 WHERE session_id='rows' AND sequence=?1",
                params![sequence, wire.to_string()],
            )
            .unwrap();
    }
    drop_v40(&connection);
    connection.execute_batch("DROP TABLE timeline_array_elements; DROP TABLE agent_store_format; CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 38)); INSERT INTO agent_store_format VALUES(38);").unwrap();
    assert_eq!(schema(&connection), schema_of(38));
    let old_events = rows(&connection, "timeline_events");
    drop(connection);
    let mut exported = Vec::new();
    dim_app_server::storage::export_timeline(database.0.clone(), "rows", &mut exported)
        .await
        .unwrap();
    let exported = String::from_utf8(exported).unwrap();
    let exported = exported
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(exported[0]["store_format"], 38);
    assert_eq!(
        exported
            .iter()
            .filter(|line| line["kind"] == "event")
            .map(|line| line["event"].clone())
            .collect::<Vec<_>>(),
        originals
    );
    let connection = Connection::open(&database.0).unwrap();
    assert_eq!(schema(&connection), schema_of(38));
    assert_eq!(rows(&connection, "timeline_events"), old_events);
    connection.execute("UPDATE timeline_events SET body=json_set(body,'$.encoding','broken') WHERE session_id='rows' AND sequence=2",[]).unwrap();
    let broken = rows(&connection, "timeline_events");
    let content = rows(&connection, "timeline_content");
    drop(connection);
    assert!(SqliteStorage::open(database.0.clone()).is_err());
    let connection = Connection::open(&database.0).unwrap();
    assert_eq!(schema(&connection), schema_of(38));
    assert_eq!(marker(&connection), [38]);
    assert_eq!(rows(&connection, "timeline_events"), broken);
    assert_eq!(rows(&connection, "timeline_content"), content);
    connection.execute("UPDATE timeline_events SET body=json_set(body,'$.encoding','event') WHERE session_id='rows' AND sequence=2",[]).unwrap();
    drop(connection);
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    assert_eq!(db.upgraded_from(), Some(38));
    let store = SqliteTimelineStore::new(db.clone());
    let events = store.read(&key("rows", "main"), 1..=2).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|record| serde_json::to_value(&record.event).unwrap())
            .collect::<Vec<_>>(),
        originals
    );
    let connection = Connection::open(&database.0).unwrap();
    assert_eq!(schema(&connection), schema_of(STORE_FORMAT));
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM timeline_array_elements WHERE session_id='rows'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        257
    );
    let locators = rows(&connection, "timeline_array_elements");
    let content = rows(&connection, "timeline_content");
    drop(connection);
    let mut exported = Vec::new();
    dim_app_server::storage::export_timeline(database.0.clone(), "rows", &mut exported)
        .await
        .unwrap();
    let exported = String::from_utf8(exported).unwrap();
    let exported = exported
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(exported[0]["store_format"], STORE_FORMAT);
    assert_eq!(
        exported
            .iter()
            .filter(|line| line["kind"] == "event")
            .map(|line| line["event"].clone())
            .collect::<Vec<_>>(),
        originals
    );
    store.close().await.unwrap();
    db.close().await.unwrap();
    drop(store);
    drop(db);
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    assert_eq!(db.upgraded_from(), None);
    let store = SqliteTimelineStore::new(db.clone());
    let connection = Connection::open(&database.0).unwrap();
    assert_eq!(rows(&connection, "timeline_array_elements"), locators);
    assert_eq!(rows(&connection, "timeline_content"), content);
    connection
        .execute(
            "DELETE FROM timeline_array_elements WHERE session_id='rows' AND element_index=100",
            [],
        )
        .unwrap();
    drop(connection);
    let scope = TimelineReadScope {
        key: key("rows", "main"),
        through: 2,
        segments: vec![TimelineReadSegment {
            branch_id: "main".into(),
            from: 1,
            through: 2,
            stored_through: 2,
            tail: vec![],
        }],
    };
    let mut q = query(&["element.value"]);
    q.filter = Some(QueryFilter::And {
        conditions: vec![
            eq("kind", json!("legacy/array")),
            eq("id", json!("event-1")),
            eq("element.index", json!(100)),
        ],
    });
    assert!(
        store.query(&scope, &q, Some(&page())).await.is_err(),
        "a selected missing locator must fail instead of returning no element"
    );
    store.branches("rows").await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn large_prompt_arrays_seek_paired_elements_and_preserve_trace_search_and_fork() {
    let mut costs = Vec::new();
    let mut page_costs = Vec::new();
    let mut count_costs = Vec::new();
    for size in [128u64, 2048] {
        let database = Database::new();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        store
            .create_session("rows", &SessionConfig::default())
            .await
            .unwrap();
        let changes = |prefix: &str| {
            (0..size)
                .map(|index| PromptChange::Set {
                    entry: PromptEntry {
                        source: format!("{prefix}-{index}"),
                        version: "1".into(),
                        body: format!(
                            "body-{prefix}-{index} {} {}",
                            "x".repeat(256),
                            if prefix == "a" && index == 2 {
                                "needleword"
                            } else {
                                "ordinary"
                            }
                        ),
                        scope: PromptScope::Branch {
                            timeline: "main".into(),
                        },
                        delivery: PromptDelivery::System,
                        order: i32::try_from(index).unwrap(),
                    },
                })
                .collect::<Vec<_>>()
        };
        let a = changes("a");
        let b = changes("b");
        let kind = |revision: &str, changes: Vec<PromptChange>| {
            EventKind::PromptChanged(PromptChangedData {
                timeline: "main".into(),
                revision: revision.into(),
                previous: None,
                reason: PromptChangeReason::Refresh,
                changes,
            })
        };
        let mut first = record(size + 1, kind("a", a.clone()));
        first.trace = Some(TimelineTrace {
            event_id: first.event.id.clone(),
            original: EventDraft {
                parent_call_id: None,
                id: None,
                tags: None,
                kind: kind("before", changes("before")),
            },
            steps: vec![TraceStep {
                handler: "replace-prompt".into(),
                result: Some(EventDraft {
                    parent_call_id: None,
                    id: None,
                    tags: None,
                    kind: first.event.kind.clone(),
                }),
            }],
        });
        let second = record(size + 2, kind("b", b.clone()));
        let mut records = (1..=size)
            .map(|sequence| {
                record(
                    sequence,
                    EventKind::Extension(
                        ExtensionEvent::new("rows", "noise", json!({"old":sequence})).unwrap(),
                    ),
                )
            })
            .collect::<Vec<_>>();
        records.extend([first.clone(), second.clone()]);
        append(&store, key("rows", "main"), records).await.unwrap();
        let read = store
            .read(&key("rows", "main"), size + 1..=size + 2)
            .await
            .unwrap();
        assert_eq!(read[0].event, first.event);
        assert_eq!(
            serde_json::to_value(&read[0].trace).unwrap(),
            serde_json::to_value(&first.trace).unwrap()
        );
        assert_eq!(read[1].event, second.event);
        let connection = Connection::open(&database.0).unwrap();
        let arrays:Vec<String>=connection.prepare("SELECT json_extract(body,'$.record.changes.id') FROM timeline_events WHERE session_id='rows' AND sequence>?1 ORDER BY sequence").unwrap().query_map([i64::try_from(size).unwrap()],|row|row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(arrays.len(), 2);
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM timeline_array_elements WHERE session_id='rows'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            i64::try_from(size * 3).unwrap()
        );
        assert!(connection.query_row("SELECT max(length(CAST(body AS BLOB))) FROM timeline_events WHERE session_id='rows'",[],|row|row.get::<_,i64>(0)).unwrap()<4096);
        drop(connection);
        let config = store.session_config_text("rows").await.unwrap().unwrap();
        store
            .fork_session("rows", "copy", size + 2, &config)
            .await
            .unwrap();
        let copied = store
            .read(&key("copy", "main"), size + 1..=size + 2)
            .await
            .unwrap();
        assert_eq!(copied[0].event, first.event);
        assert_eq!(
            serde_json::to_value(&copied[0].trace).unwrap(),
            serde_json::to_value(&first.trace).unwrap()
        );
        assert_eq!(copied[1].event, second.event);
        let scope = TimelineReadScope {
            key: key("rows", "main"),
            through: size + 2,
            segments: vec![TimelineReadSegment {
                branch_id: "main".into(),
                from: 1,
                through: size + 2,
                stored_through: size + 2,
                tail: vec![],
            }],
        };
        let mut search = query(&["id"]);
        search.rows = None;
        search.filter = Some(QueryFilter::And {
            conditions: vec![
                eq("kind", json!("prompt/changed")),
                QueryFilter::TextCandidate {
                    field: "data".into(),
                    partition_by: vec![],
                    terms: vec!["needleword".into()],
                },
            ],
        });
        for session in ["rows", "copy"] {
            let mut search_scope = scope.clone();
            search_scope.key.session_id = session.into();
            let QueryOutput::Rows { rows } = store
                .query(&search_scope, &search, Some(&page()))
                .await
                .unwrap()
                .result
            else {
                panic!("search identities")
            };
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows[0].values[0],
                QueryValue::Value {
                    value: json!(first.event.id)
                }
            );
        }
        store.rollback_session("copy").await.unwrap();
        let connection = Connection::open(&database.0).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM timeline_array_elements WHERE session_id='copy'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        connection.execute("UPDATE timeline_content SET body='{}' WHERE session_id='rows' AND kind='array-element' AND hash NOT IN (SELECT content_ref FROM timeline_array_elements WHERE session_id='rows' AND ((array_id=?1 AND element_index=2) OR (array_id=?2 AND element_index=7)))",params![arrays[0],arrays[1]]).unwrap();
        drop(connection);
        store.branches("rows").await.unwrap();
        let mut q = query(&["id", "element.index", "element.value"]);
        q.filter = Some(QueryFilter::And {
            conditions: vec![
                eq("kind", json!("prompt/changed")),
                QueryFilter::Or {
                    conditions: vec![
                        QueryFilter::And {
                            conditions: vec![
                                eq("id", json!(first.event.id)),
                                eq("element.index", json!(2)),
                            ],
                        },
                        QueryFilter::And {
                            conditions: vec![
                                eq("id", json!(second.event.id)),
                                eq("element.index", json!(7)),
                            ],
                        },
                    ],
                },
            ],
        });
        store.take_node_loads();
        store.record_query_statistics();
        let reply = store.query(&scope, &q, Some(&page())).await.unwrap();
        assert!(reply.next.is_none());
        let QueryOutput::Rows { rows } = reply.result else {
            panic!("element rows")
        };
        assert_eq!(
            rows.iter()
                .map(|row| row.values.clone())
                .collect::<Vec<_>>(),
            vec![
                vec![
                    QueryValue::Value {
                        value: json!(first.event.id)
                    },
                    QueryValue::Value { value: json!(2) },
                    QueryValue::Value {
                        value: serde_json::to_value(&a[2]).unwrap()
                    }
                ],
                vec![
                    QueryValue::Value {
                        value: json!(second.event.id)
                    },
                    QueryValue::Value { value: json!(7) },
                    QueryValue::Value {
                        value: serde_json::to_value(&b[7]).unwrap()
                    }
                ],
            ]
        );
        assert_eq!(
            store.take_node_loads().loads,
            2,
            "only the paired contents are decoded"
        );
        let stats = store.take_query_statistics();
        assert!(
            stats
                .iter()
                .flat_map(|stat| &stat.plan)
                .any(|line| line.contains("timeline_events_by_id") && line.contains("event_id=?")),
            "{stats:?}"
        );
        costs.push(stats.iter().map(|stat| stat.vm_steps).sum::<i32>());
        let mut metadata = query(&["element.index"]);
        metadata.filter = Some(QueryFilter::And {
            conditions: vec![
                eq("kind", json!("prompt/changed")),
                eq("id", json!(first.event.id)),
            ],
        });
        let mut measured = Vec::new();
        for direction in [
            TimelineQueryDirection::Ascending,
            TimelineQueryDirection::Descending,
        ] {
            metadata.order_by = vec![QueryOrder {
                key: QueryOrderKey::Position,
                direction,
            }];
            let mut paging = QueryPage {
                limit: NonZeroU64::new(16).unwrap(),
                after: None,
            };
            let mut offset = 0;
            store.take_node_loads();
            loop {
                store.record_query_statistics();
                let result = store.query(&scope, &metadata, Some(&paging)).await.unwrap();
                let stats = store.take_query_statistics();
                let QueryOutput::Rows { rows } = result.result else {
                    panic!("metadata page")
                };
                assert_eq!(rows.len(), 16);
                let index = if direction == TimelineQueryDirection::Ascending {
                    offset
                } else {
                    size - 1 - offset
                };
                assert_eq!(
                    rows[0].values[0],
                    QueryValue::Value {
                        value: json!(index)
                    }
                );
                if offset < 48 || result.next.is_none() {
                    measured.push(stats.iter().map(|stat| stat.vm_steps).sum::<i32>());
                }
                offset += 16;
                let Some(next) = result.next else { break };
                paging.after = Some(next);
            }
            assert_eq!(offset, size);
            assert_eq!(
                store.take_node_loads().loads,
                0,
                "index-only pages do not decode element bodies"
            );
        }
        metadata.select = QuerySelect::Exists;
        metadata.order_by.clear();
        assert!(matches!(
            store.query(&scope, &metadata, None).await.unwrap().result,
            QueryOutput::Exists { exists: true }
        ));
        assert_eq!(store.take_node_loads().loads, 0);
        metadata.select = QuerySelect::Count;
        store.record_query_statistics();
        assert!(
            matches!(store.query(&scope,&metadata,None).await.unwrap().result,QueryOutput::Count {count} if count==size)
        );
        assert_eq!(store.take_node_loads().loads, 0);
        count_costs.push(
            store
                .take_query_statistics()
                .iter()
                .map(|stat| stat.vm_steps)
                .sum::<i32>(),
        );
        page_costs.push(measured);
        let filter = q.filter.take().unwrap();
        q.filter = Some(QueryFilter::And {
            conditions: vec![
                filter,
                QueryFilter::Prefix {
                    value: QueryOperand::Field {
                        path: "element.value.entry.body".into(),
                    },
                    prefix: "body-".into(),
                },
            ],
        });
        store.take_node_loads();
        let QueryOutput::Rows { rows } =
            store.query(&scope, &q, Some(&page())).await.unwrap().result
        else {
            panic!("verified elements")
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(store.take_node_loads().loads, 2);
        let connection = Connection::open(&database.0).unwrap();
        connection.execute("UPDATE timeline_content SET body='[]' WHERE session_id='rows' AND hash=(SELECT content_ref FROM timeline_array_elements WHERE session_id='rows' AND array_id=?1 AND element_index=2)",[&arrays[0]]).unwrap();
        drop(connection);
        assert!(
            store.query(&scope, &q, Some(&page())).await.is_err(),
            "selected content is always hash checked"
        );
        store.close().await.unwrap();
        db.close().await.unwrap();
    }
    assert!(
        costs[1] < costs[0] * 2,
        "point work must not grow with old events or the source array: {costs:?}"
    );
    for (small, large) in page_costs[0].iter().zip(&page_costs[1]) {
        assert!(
            *large < small * 2,
            "late pages must seek their element index: {page_costs:?}"
        );
    }
    assert!(
        count_costs[1] < count_costs[0] * 2,
        "counts use the array length: {count_costs:?}"
    );
}

async fn parity(
    store: &SqliteTimelineStore,
    memory: &MemoryTimelineStore,
    scope: &TimelineReadScope,
    query: &QuerySpec,
) -> Vec<dim_agent_runtime::QueryRow> {
    let scalar = query.group_by.is_empty()
        && matches!(
            query.select,
            QuerySelect::Count | QuerySelect::Exists | QuerySelect::Characters { .. }
        );
    let mut page = (!scalar).then(page);
    query.validate(page.as_ref()).unwrap();
    let mut all = Vec::new();
    loop {
        let expected = memory.query(scope, query, page.as_ref()).await.unwrap();
        let actual = store.query(scope, query, page.as_ref()).await.unwrap();
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "{query:?}"
        );
        if let QueryOutput::Rows { rows } = actual.result {
            all.extend(rows);
        }
        let Some(next) = actual.next else { break };
        page.as_mut().unwrap().after = Some(next);
    }
    all
}

#[tokio::test]
async fn element_rows_keep_indices_groups_and_event_relations_across_pages_and_publication() {
    for padding in [String::new(), "x".repeat(5000)] {
        let database = Database::new();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        let memory = MemoryTimelineStore::default();
        store
            .create_session("rows", &SessionConfig::default())
            .await
            .unwrap();
        memory
            .create_session("rows", &SessionConfig::default())
            .await
            .unwrap();
        let values = [
            (
                "array",
                json!({"changes":[{"key":"a","value":"first","refs":[{"key":"r"}],"padding":padding},null,{"key":"b","value":"中😀"},{"key":"a","value":"later"}]}),
            ),
            ("array", json!({"changes":null})),
            (
                "terminal",
                json!({"key":"a","number":1.0,"refs":[{"key":"b"}]}),
            ),
            ("array", json!({"changes":[]})),
            (
                "array",
                json!({"changes":[false,1,1.0,"text",{"key":"b","value":"new"}]}),
            ),
            ("invalid", json!({"changes":"not an array"})),
            ("array", json!({})),
        ];
        let records: Vec<_> = values
            .into_iter()
            .enumerate()
            .map(|(index, (name, data))| {
                record(
                    index as u64 + 1,
                    EventKind::Extension(ExtensionEvent::new("rows", name, data).unwrap()),
                )
            })
            .collect();
        append(&store, key("rows", "main"), records[..3].to_vec())
            .await
            .unwrap();
        memory
            .append(
                key("rows", "main"),
                records[..3].to_vec(),
                Box::new(|result| result.unwrap()),
            )
            .unwrap();
        let mut scope = TimelineReadScope {
            key: key("rows", "main"),
            through: 7,
            segments: vec![TimelineReadSegment {
                branch_id: "main".into(),
                from: 1,
                through: 7,
                stored_through: 3,
                tail: records[3..]
                    .iter()
                    .map(|record| record.event.clone())
                    .collect(),
            }],
        };
        let mut q = query(&[
            "position",
            "id",
            "element.index",
            "element.value",
            "element.value.key",
            "element.value.refs[].key",
        ]);
        let rows = parity(&store, &memory, &scope, &q).await;
        assert_eq!(rows.len(), 9);
        assert_eq!(rows[1].values[3], QueryValue::Value { value: Value::Null });
        assert_eq!(rows[1].values[4], QueryValue::Missing);
        q.order_by = vec![QueryOrder {
            key: QueryOrderKey::Position,
            direction: TimelineQueryDirection::Descending,
        }];
        parity(&store, &memory, &scope, &q).await;
        q.order_by.clear();
        q.group_by = vec!["element.value.key".into()];
        for pick in [QueryPick::First, QueryPick::Last] {
            q.pick = Some(pick);
            parity(&store, &memory, &scope, &q).await;
        }
        q.pick = None;
        q.select = QuerySelect::Count;
        parity(&store, &memory, &scope, &q).await;
        q.group_by.clear();
        parity(&store, &memory, &scope, &q).await;
        assert!(matches!(
            store.query(&scope, &q, None).await.unwrap().result,
            QueryOutput::Count { count: 9 }
        ));
        q.select = QuerySelect::Characters {
            field: "element.value.value".into(),
        };
        parity(&store, &memory, &scope, &q).await;
        assert!(matches!(
            store.query(&scope, &q, None).await.unwrap().result,
            QueryOutput::Count { count: 15 }
        ));
        q.select = QuerySelect::Exists;
        parity(&store, &memory, &scope, &q).await;
        q = query(&["id", "element.index", "element.value.key"]);
        for (inner, outer, expected) in [
            ("data.key", "element.value.key", 2),
            ("data.number", "element.value", 2),
        ] {
            q.filter = Some(QueryFilter::And {
                conditions: vec![
                    eq("kind", json!("rows/array")),
                    QueryFilter::Exists {
                        condition: Box::new(QueryFilter::And {
                            conditions: vec![
                                eq("kind", json!("rows/terminal")),
                                QueryFilter::Compare {
                                    left: QueryOperand::Field { path: inner.into() },
                                    comparison: QueryComparison::Eq,
                                    right: QueryOperand::Outer { path: outer.into() },
                                },
                            ],
                        }),
                    },
                ],
            });
            assert_eq!(parity(&store, &memory, &scope, &q).await.len(), expected);
        }
        q.filter = Some(QueryFilter::And {
            conditions: vec![
                eq("kind", json!("rows/array")),
                QueryFilter::Any {
                    field: "element.value.refs".into(),
                    condition: Box::new(QueryFilter::Compare {
                        left: QueryOperand::Element { path: "key".into() },
                        comparison: QueryComparison::Eq,
                        right: QueryOperand::Value { value: json!("r") },
                    }),
                },
            ],
        });
        assert_eq!(parity(&store, &memory, &scope, &q).await.len(), 1);
        q.filter = Some(QueryFilter::And {
            conditions: vec![
                eq("kind", json!("rows/array")),
                QueryFilter::Exists {
                    condition: Box::new(QueryFilter::And {
                        conditions: vec![
                            eq("kind", json!("rows/terminal")),
                            QueryFilter::Any {
                                field: "data.refs".into(),
                                condition: Box::new(QueryFilter::Compare {
                                    left: QueryOperand::Element { path: "key".into() },
                                    comparison: QueryComparison::Eq,
                                    right: QueryOperand::Outer {
                                        path: "element.value.key".into(),
                                    },
                                }),
                            },
                        ],
                    }),
                },
            ],
        });
        assert_eq!(parity(&store, &memory, &scope, &q).await.len(), 2);
        q.filter = Some(QueryFilter::And {
            conditions: vec![
                eq("kind", json!("rows/array")),
                QueryFilter::Or {
                    conditions: vec![
                        QueryFilter::And {
                            conditions: vec![
                                eq("id", json!("event-1")),
                                eq("element.index", json!(0)),
                            ],
                        },
                        QueryFilter::And {
                            conditions: vec![
                                eq("id", json!("event-5")),
                                eq("element.index", json!(4)),
                            ],
                        },
                    ],
                },
            ],
        });
        assert_eq!(parity(&store, &memory, &scope, &q).await.len(), 2);
        q = query(&["id", "element.index", "element.value"]);
        let first = store.query(&scope, &q, Some(&page())).await.unwrap();
        append(&store, key("rows", "main"), records[3..].to_vec())
            .await
            .unwrap();
        memory
            .append(
                key("rows", "main"),
                records[3..].to_vec(),
                Box::new(|result| result.unwrap()),
            )
            .unwrap();
        scope.segments[0].stored_through = 7;
        scope.segments[0].tail.clear();
        let continued = QueryPage {
            after: first.next,
            ..page()
        };
        let actual = store.query(&scope, &q, Some(&continued)).await.unwrap();
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(memory.query(&scope, &q, Some(&continued)).await.unwrap())
                .unwrap()
        );
        let QueryOutput::Rows { rows } = actual.result else {
            panic!("element rows")
        };
        assert_eq!(rows[0].values[1], QueryValue::Value { value: json!(2) });
        parity(&store, &memory, &scope, &q).await;
        q.rows = Some(QueryRows::Elements {
            field: "tags".into(),
        });
        parity(&store, &memory, &scope, &q).await;
        q.rows = Some(QueryRows::Elements {
            field: "data.changes".into(),
        });
        q.filter = Some(eq("kind", json!("rows/invalid")));
        let error = store.query(&scope, &q, Some(&page())).await;
        assert!(
            matches!(error, Err(AgentError::InvalidInput(_))),
            "{error:?}"
        );
        assert!(matches!(
            memory.query(&scope, &q, Some(&page())).await,
            Err(AgentError::InvalidInput(_))
        ));
        q = query(&["id"]);
        q.rows = None;
        let old = store.query(&scope, &q, Some(&page())).await.unwrap();
        q.rows = Some(QueryRows::Elements {
            field: "data.changes".into(),
        });
        let foreign = QueryPage {
            after: old.next,
            ..page()
        };
        assert!(store.query(&scope, &q, Some(&foreign)).await.is_err());
        assert!(memory.query(&scope, &q, Some(&foreign)).await.is_err());
        let original = r#"{"filter":null,"select":{"kind":"fields","fields":["id"]},"group_by":[],"pick":null,"order_by":[]}"#;
        let original: QuerySpec = serde_json::from_str(original).unwrap();
        assert_eq!(
            serde_json::to_string(&original).unwrap(),
            r#"{"filter":null,"select":{"kind":"fields","fields":["id"]},"group_by":[],"pick":null,"order_by":[]}"#
        );
        let create = record(
            8,
            EventKind::BranchCreate(BranchCreateData {
                at: BranchCreateAt::Sequence { sequence: 1 },
                source: BranchSource::Host {
                    operation_id: "rows-child".into(),
                },
                branch: "child".into(),
                prompt: "child".into(),
            }),
        );
        append(&store, key("rows", "main"), vec![create.clone()])
            .await
            .unwrap();
        memory
            .append(
                key("rows", "main"),
                vec![create],
                Box::new(|result| result.unwrap()),
            )
            .unwrap();
        let child = record(
            1,
            EventKind::Extension(
                ExtensionEvent::new(
                    "rows",
                    "array",
                    json!({"changes":[{"key":"child"},{"key":"a"}]}),
                )
                .unwrap(),
            ),
        );
        append(&store, key("rows", "child"), vec![child.clone()])
            .await
            .unwrap();
        memory
            .append(
                key("rows", "child"),
                vec![child],
                Box::new(|result| result.unwrap()),
            )
            .unwrap();
        let chain = TimelineReadScope {
            key: key("rows", "child"),
            through: 1,
            segments: vec![
                TimelineReadSegment {
                    branch_id: "main".into(),
                    from: 1,
                    through: 1,
                    stored_through: 1,
                    tail: vec![],
                },
                TimelineReadSegment {
                    branch_id: "child".into(),
                    from: 1,
                    through: 1,
                    stored_through: 1,
                    tail: vec![],
                },
            ],
        };
        let all = query(&["position", "element.index", "element.value"]);
        assert_eq!(parity(&store, &memory, &chain, &all).await.len(), 6);
        store.close().await.unwrap();
        db.close().await.unwrap();
    }
}

#[tokio::test]
async fn element_pages_preserve_paired_indices_across_projection_batches() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    store
        .create_session("rows", &SessionConfig::default())
        .await
        .unwrap();
    let changes: Vec<_> = (0..300).collect();
    append(
        &store,
        key("rows", "main"),
        vec![record(
            1,
            EventKind::Extension(
                ExtensionEvent::new("rows", "array", json!({"changes":changes})).unwrap(),
            ),
        )],
    )
    .await
    .unwrap();
    let scope = TimelineReadScope {
        key: key("rows", "main"),
        through: 1,
        segments: vec![TimelineReadSegment {
            branch_id: "main".into(),
            from: 1,
            through: 1,
            stored_through: 1,
            tail: vec![],
        }],
    };
    let q = query(&["element.index", "element.value"]);
    let mut page = QueryPage {
        limit: NonZeroU64::new(150).unwrap(),
        after: None,
    };
    let mut count = 0;
    loop {
        let result = store.query(&scope, &q, Some(&page)).await.unwrap();
        let QueryOutput::Rows { rows } = result.result else {
            panic!("element rows")
        };
        for row in rows {
            assert_eq!(
                row.values,
                vec![
                    QueryValue::Value {
                        value: json!(count)
                    },
                    QueryValue::Value {
                        value: json!(count)
                    }
                ]
            );
            count += 1;
        }
        let Some(next) = result.next else { break };
        page.after = Some(next);
    }
    assert_eq!(count, 300);
    store.close().await.unwrap();
    db.close().await.unwrap();
}
