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
            json!({"changes":[{"key":"a","value":"first","refs":[{"key":"r"}]},null,{"key":"b","value":"中😀"},{"key":"a","value":"later"}]}),
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
                        conditions: vec![eq("id", json!("event-1")), eq("element.index", json!(0))],
                    },
                    QueryFilter::And {
                        conditions: vec![eq("id", json!("event-5")), eq("element.index", json!(4))],
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
        serde_json::to_value(memory.query(&scope, &q, Some(&continued)).await.unwrap()).unwrap()
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
