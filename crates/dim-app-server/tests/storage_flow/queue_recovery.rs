use super::*;
use dim_agent_runtime::{
    InputSnapshot, QueryFilter, QueryOperand, QueryOutput, QueryPage, QueryPick, QuerySelect,
    QuerySpec, QueryValue, TimelineReadScope, TimelineReadSegment,
};
use serde_json::{Value, json};

fn builder(store: &Arc<SqliteTimelineStore>) -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    builder.timeline_store(store.clone());
    builder
}

#[tokio::test]
async fn owner_result_and_source_queries_skip_the_growing_completed_prefix() {
    let mut costs = Vec::new();
    for size in [600, 6000] {
        let database = Database::new();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        store
            .create_session("properties", &SessionConfig::default())
            .await
            .unwrap();
        let mut records = Vec::new();
        for base in (0..size).step_by(4) {
            let source = record(
                base + 1,
                EventKind::InputMessage(InputMessageData::text(
                    "completed",
                    Some(InputMode::Append),
                )),
            );
            let input_id = source.event.id.clone();
            records.extend([
                source,
                record(base+2,EventKind::InputApplied(InputAppliedData { input_id: input_id.clone(),mode:Some(InputMode::Append) })),
                record(base+3,EventKind::InputConsumed(InputConsumedData { input_id })),
                record(base+4,EventKind::Extension(ExtensionEvent::new("input-queue","recovered",json!({"schema":"input-queue/v1","source_through":base+3,"state":{"entries_root":null,"queued":0,"turn":null,"starting":false}})).unwrap())),
            ]);
        }
        records.extend((size + 1..=size + 2).map(|sequence| {
            record(
                sequence,
                EventKind::InputMessage(InputMessageData::text(
                    "new source",
                    Some(InputMode::Append),
                )),
            )
        }));
        append(&store, key("properties", "main"), records)
            .await
            .unwrap();
        let scope = TimelineReadScope {
            key: key("properties", "main"),
            through: size + 2,
            segments: vec![TimelineReadSegment {
                branch_id: "main".into(),
                from: 1,
                through: size + 2,
                stored_through: size + 2,
                tail: vec![],
            }],
        };
        let mut query = QuerySpec {
            rows: None,
            filter: Some(QueryFilter::In {
                value: QueryOperand::Field {
                    path: "kind".into(),
                },
                values: vec![json!("input-queue/recovered")],
            }),
            select: QuerySelect::Fields {
                fields: vec!["position.sequence".into(), "data".into()],
            },
            group_by: vec![],
            pick: Some(QueryPick::Last),
            order_by: vec![],
        };
        let page = QueryPage {
            limit: std::num::NonZeroU64::new(128).unwrap(),
            after: None,
        };
        store.record_query_statistics();
        let QueryOutput::Rows { rows } = store
            .query(&scope, &query, Some(&page))
            .await
            .unwrap()
            .result
        else {
            panic!("result fields")
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].values[0], QueryValue::Value { value: json!(size) });
        let latest = store
            .take_query_statistics()
            .iter()
            .map(|stat| stat.vm_steps)
            .sum::<i32>();
        query.pick = None;
        query.filter = Some(QueryFilter::In {
            value: QueryOperand::Field {
                path: "kind".into(),
            },
            values: [
                "input/message",
                "input/consumed",
                "input/withdraw",
                "turn/start",
                "turn/end",
                "stop/interrupt",
            ]
            .map(Value::from)
            .to_vec(),
        });
        query.select = QuerySelect::Fields {
            fields: [
                "kind",
                "id",
                "position.sequence",
                "data.mode",
                "data.input_id",
                "data.turn_id",
                "data.handover.turn_id",
            ]
            .map(String::from)
            .to_vec(),
        };
        let mut suffix = scope.clone();
        suffix.segments[0].from = size;
        store.record_query_statistics();
        let result = store.query(&suffix, &query, Some(&page)).await.unwrap();
        let QueryOutput::Rows { rows } = result.result else {
            panic!("source fields")
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter()
                .map(|row| row.values[2].clone())
                .collect::<Vec<_>>(),
            [size + 1, size + 2].map(|sequence| QueryValue::Value {
                value: json!(sequence)
            }),
        );
        assert!(result.next.is_none());
        let new_sources = store
            .take_query_statistics()
            .iter()
            .map(|stat| stat.vm_steps)
            .sum::<i32>();
        suffix.through = size;
        suffix.segments[0].through = size;
        suffix.segments[0].stored_through = size;
        store.record_query_statistics();
        let QueryOutput::Rows { rows } = store
            .query(&suffix, &query, Some(&page))
            .await
            .unwrap()
            .result
        else {
            panic!("empty source fields")
        };
        assert!(rows.is_empty());
        let empty = store
            .take_query_statistics()
            .iter()
            .map(|stat| stat.vm_steps)
            .sum::<i32>();
        costs.push([latest, new_sources, empty]);
        store.close().await.unwrap();
        db.close().await.unwrap();
    }
    for (small, large) in costs[0].iter().zip(costs[1]) {
        assert!(
            large <= small * 2,
            "completed history must not be scanned: {costs:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_resume_finishes_multiple_source_pages_without_republishing_an_older_count() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let agent = MinimalHost::start(&builder(&store)).await.unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    session
        .submit("initial result".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    settled("the initial result is complete", session.idle("main"))
        .await
        .unwrap();
    let id = session.id().to_owned();
    agent.close().await.unwrap();
    let head = store.head(&key(&id, "main")).await.unwrap();
    let records = (1..=300)
        .map(|offset| {
            record(
                head + offset,
                EventKind::InputMessage(InputMessageData::text(
                    "not processed before close",
                    Some(InputMode::Append),
                )),
            )
        })
        .collect();
    append(&store, key(&id, "main"), records).await.unwrap();
    for _ in 0..2 {
        let agent = MinimalHost::start(&builder(&store)).await.unwrap();
        let session = settled(
            "all new source pages restore",
            agent.restore_session(id.clone()),
        )
        .await
        .unwrap();
        let mut inventory = session.on::<InputSnapshot>("main").unwrap().unwrap();
        assert_eq!(inventory.borrow().queued, 300);
        let idle = settled("recovery results are dispatched", session.idle("main"));
        tokio::pin!(idle);
        loop {
            tokio::select! {
                changed = inventory.changed() => {
                    changed.unwrap();
                    assert_eq!(inventory.borrow_and_update().queued, 300, "an earlier recovery result must not regress the restored inventory");
                }
                result = &mut idle => {
                    result.unwrap();
                    break;
                }
            }
        }
        let history = chain_events(&session, "main", ..).await.unwrap();
        assert_eq!(history.iter().filter(|event| matches!(&event.kind, EventKind::InputApplied(data) if data.input_id.starts_with("event-"))).count(), 300, "source pages apply once across repeated resumes");
        agent.close().await.unwrap();
    }
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_nodes_restore_pending_inputs_and_remove_them_when_a_turn_consumes_them() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let agent = MinimalHost::start(&builder(&store)).await.unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let id = session.id().to_owned();
    let mut inputs = Vec::new();
    for _ in 0..257 {
        inputs.push(
            session
                .input(
                    InputMessageData::text("kept across resume", Some(InputMode::Append)),
                    "main",
                )
                .await
                .unwrap()
                .id
                .clone(),
        );
    }
    settled("all append inputs are published", session.idle("main"))
        .await
        .unwrap();
    assert_eq!(
        session
            .get::<InputSnapshot>("main")
            .unwrap()
            .unwrap()
            .queued,
        257
    );
    agent.close().await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();

    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let agent = MinimalHost::start(&builder(&store)).await.unwrap();
    let session = settled(
        "the current queue nodes restore",
        agent.restore_session(id.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        session
            .get::<InputSnapshot>("main")
            .unwrap()
            .unwrap()
            .queued,
        257
    );
    session
        .submit("consume restored inputs".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    settled("the restored entries are consumed", session.idle("main"))
        .await
        .unwrap();
    assert_eq!(
        session
            .get::<InputSnapshot>("main")
            .unwrap()
            .unwrap()
            .queued,
        0
    );
    let history = chain_events(&session, "main", ..).await.unwrap();
    let consumed: Vec<_> = history
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::InputConsumed(data) if inputs.contains(&data.input_id) => {
                Some(data.input_id.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(consumed, inputs);
    agent.close().await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_resume_reuses_an_application_written_by_an_older_owner_after_a_saved_result() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let agent = MinimalHost::start(&builder(&store)).await.unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    session
        .submit("save a compatible result".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    settled("the first result is saved", session.idle("main"))
        .await
        .unwrap();
    let id = session.id().to_owned();
    agent.close().await.unwrap();
    let head = store.head(&key(&id, "main")).await.unwrap();
    let source = record(
        head + 1,
        EventKind::InputMessage(InputMessageData::text(
            "older owner output",
            Some(InputMode::Queue),
        )),
    );
    let input_id = source.event.id.clone();
    append(
        &store,
        key(&id, "main"),
        vec![
            source,
            record(
                head + 2,
                EventKind::InputApplied(InputAppliedData {
                    input_id: input_id.clone(),
                    mode: Some(InputMode::Queue),
                }),
            ),
        ],
    )
    .await
    .unwrap();
    let agent = MinimalHost::start(&builder(&store)).await.unwrap();
    let session = settled(
        "saved state and older output restore",
        agent.restore_session(id),
    )
    .await
    .unwrap();
    settled("the older application completes", session.idle("main"))
        .await
        .unwrap();
    let history = chain_events(&session, "main", ..).await.unwrap();
    assert_eq!(history.iter().filter(|event| matches!(&event.kind, EventKind::InputApplied(data) if data.input_id == input_id)).count(), 1);
    assert_eq!(history.iter().filter(|event| matches!(&event.kind, EventKind::InputConsumed(data) if data.input_id == input_id)).count(), 1);
    agent.close().await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_restore_rejects_missing_cyclic_and_invalid_nodes_without_history_fallback() {
    for broken in ["missing", "cyclic", "summary"] {
        let database = Database::new();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        let agent = MinimalHost::start(&builder(&store)).await.unwrap();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        let input = session
            .submit("save state".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        settled("the original queue completes", session.idle("main"))
            .await
            .unwrap();
        let id = session.id().to_owned();
        agent.close().await.unwrap();
        let head = store.head(&key(&id, "main")).await.unwrap();
        let node_id = "broken-queue-node";
        let mut records = Vec::new();
        if broken != "missing" {
            let mut node = record(head + 1, EventKind::Extension(ExtensionEvent::new("input-queue", "node", json!({
                "entry":{"input_id": input.id, "sequence": input.sequence,"kind":{"kind":"input","mode":"append","released":true}},
                "left":if broken == "cyclic" { Some(node_id) } else { None }, "right":null,
                "height":if broken == "summary" { 4 } else { 1 },"queued":1,"handovers":0,"first_held":null
            })).unwrap()));
            Arc::make_mut(&mut node.event).id = node_id.into();
            records.push(node);
        }
        let end = head + records.len() as u64 + 1;
        records.push(record(
            end,
            EventKind::Extension(
                ExtensionEvent::new(
                    "input-queue",
                    "recovered",
                    json!({
                        "schema":"input-queue/v1","source_through":head,
                        "state":{"entries_root":node_id,"queued":1,"turn":null,"starting":false}
                    }),
                )
                .unwrap(),
            ),
        ));
        append(&store, key(&id, "main"), records).await.unwrap();
        let agent = MinimalHost::start(&builder(&store)).await.unwrap();
        let failure = settled(
            "damaged queue state rejects restore",
            agent.restore_session(id.clone()),
        )
        .await
        .err()
        .expect("a bad root must not use older state");
        assert!(failure.to_string().contains(broken), "{broken}: {failure}");
        assert!(agent.get_session(&id).is_none());
        let kept = store.read(&key(&id, "main"), end..=end).await.unwrap();
        assert_eq!(kept[0].event.kind.key(), "input-queue/recovered");
        agent.close().await.unwrap();
        store.close().await.unwrap();
        db.close().await.unwrap();
    }
}

#[tokio::test]
async fn owner_node_identity_queries_seek_ids_instead_of_scanning_old_node_history() {
    let mut costs = Vec::new();
    for n in [600_u64, 6_000] {
        let database = Database::new();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        store
            .create_session("node-lookup", &SessionConfig::default())
            .await
            .unwrap();
        let records = (1..=n)
            .map(|sequence| {
                record(
                    sequence,
                    EventKind::Extension(
                        ExtensionEvent::new("owner", "node", json!({"value":sequence})).unwrap(),
                    ),
                )
            })
            .collect();
        append(&store, key("node-lookup", "main"), records)
            .await
            .unwrap();
        let scope = TimelineReadScope {
            key: key("node-lookup", "main"),
            through: n,
            segments: vec![TimelineReadSegment {
                branch_id: "main".into(),
                from: 1,
                through: n,
                stored_through: n,
                tail: vec![],
            }],
        };
        let ids = [1, n / 2, n]
            .map(|sequence| json!(format!("event-{sequence}")))
            .to_vec();
        let query = QuerySpec {
            rows: None,
            filter: Some(QueryFilter::And {
                conditions: vec![
                    QueryFilter::In {
                        value: QueryOperand::Field {
                            path: "kind".into(),
                        },
                        values: vec![json!("owner/node")],
                    },
                    QueryFilter::In {
                        value: QueryOperand::Field { path: "id".into() },
                        values: ids.clone(),
                    },
                ],
            }),
            select: QuerySelect::Fields {
                fields: vec!["id".into(), "data.value".into()],
            },
            group_by: vec![],
            pick: None,
            order_by: vec![],
        };
        let page = QueryPage {
            limit: std::num::NonZeroU64::new(3).unwrap(),
            after: None,
        };
        store.record_query_statistics();
        let result = store.query(&scope, &query, Some(&page)).await.unwrap();
        let QueryOutput::Rows { rows } = result.result else {
            panic!("node fields")
        };
        assert_eq!(rows.len(), 3);
        for (row, id) in rows.iter().zip(&ids) {
            assert_eq!(row.values[0], QueryValue::Value { value: id.clone() });
        }
        let stats = store.take_query_statistics();
        assert!(
            stats.iter().any(|statement| statement.stage == "selection"
                && statement
                    .plan
                    .iter()
                    .any(|line| line.contains("timeline_events_by_id")
                        && line.contains("event_id=?"))),
            "{stats:?}"
        );
        costs.push(
            stats
                .iter()
                .map(|statement| statement.vm_steps as u64)
                .sum::<u64>(),
        );
        let mut streamed = vec![text(n + 1, "one"), text(n + 2, "two")];
        for record in &mut streamed {
            record.batch_end = n + 2;
        }
        append(&store, key("node-lookup", "main"), streamed)
            .await
            .unwrap();
        let scope = TimelineReadScope {
            through: n + 2,
            segments: vec![TimelineReadSegment {
                branch_id: "main".into(),
                from: 1,
                through: n + 2,
                stored_through: n + 2,
                tail: vec![],
            }],
            ..scope
        };
        let mut mixed = query;
        mixed.filter = Some(QueryFilter::And {
            conditions: vec![
                QueryFilter::In {
                    value: QueryOperand::Field {
                        path: "kind".into(),
                    },
                    values: vec![json!("owner/node"), json!("step/text")],
                },
                QueryFilter::In {
                    value: QueryOperand::Field { path: "id".into() },
                    values: vec![json!("event-1"), json!(format!("event-{}", n + 2))],
                },
            ],
        });
        let QueryOutput::Rows { rows } = store
            .query(&scope, &mixed, Some(&page))
            .await
            .unwrap()
            .result
        else {
            panic!("mixed fields")
        };
        assert_eq!(rows.len(), 2, "merged output identities remain queryable");
        store.close().await.unwrap();
        db.close().await.unwrap();
    }
    assert!(
        costs[1] <= costs[0] * 2,
        "fixed identity reads must not grow with old nodes: {costs:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_results_commit_with_outputs_and_resume_once_after_a_failed_commit() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    builder.timeline_store(store.clone());
    let agent = MinimalHost::start(&builder).await.unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let id = session.id().to_owned();
    let connection = Connection::open(&database.0).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_queue_result BEFORE INSERT ON timeline_events WHEN NEW.kind = 'input-queue/recovered' BEGIN SELECT RAISE(ABORT, 'injected queue result failure'); END").unwrap();
    let input = session
        .submit("recover this input".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    let failure = settled("queue commit failure closes its Session", session.closed())
        .await
        .unwrap_err();
    assert!(
        failure
            .to_string()
            .contains("injected queue result failure"),
        "{failure}"
    );
    let history = chain_events(&session, "main", ..).await.unwrap();
    assert!(history.iter().any(|event| event.id == input.id));
    assert!(!history.iter().any(|event| matches!(&event.kind,
        EventKind::InputApplied(data) if data.input_id == input.id)));
    connection
        .execute_batch("DROP TRIGGER fail_queue_result")
        .unwrap();
    drop(connection);
    let restored = settled(
        "the queue restores its unprocessed source",
        agent.restore_session(id.clone()),
    )
    .await
    .unwrap();
    settled("the restored input completes", restored.idle("main"))
        .await
        .unwrap();
    let head = store.head(&key(&id, "main")).await.unwrap();
    let records = store.read(&key(&id, "main"), 1..=head).await.unwrap();
    let applied: Vec<_> = records
        .iter()
        .filter(|record| {
            matches!(&record.event.kind,
        EventKind::InputApplied(data) if data.input_id == input.id)
        })
        .collect();
    assert_eq!(applied.len(), 1);
    let batch = records
        .iter()
        .find(|record| record.event.sequence == applied[0].batch_end)
        .unwrap();
    let EventKind::Extension(data) = &batch.event.kind else {
        panic!("application batch has no recovery result")
    };
    assert_eq!(batch.event.kind.key(), "input-queue/recovered");
    assert_eq!(data.data()["schema"], "input-queue/v1");
    assert!(data.data()["source_through"].as_u64().unwrap() >= input.sequence);
    assert!(
        history
            .iter()
            .all(|event| event.sequence < applied[0].event.sequence)
    );
    agent.close().await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();

    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    builder.timeline_store(store.clone());
    let agent = MinimalHost::start(&builder).await.unwrap();
    let restored = settled(
        "the queue restores its saved state",
        agent.restore_session(id.clone()),
    )
    .await
    .unwrap();
    settled("saved queue restore is idle", restored.idle("main"))
        .await
        .unwrap();
    let history = chain_events(&restored, "main", ..).await.unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(&event.kind,
        EventKind::InputApplied(data) if data.input_id == input.id))
            .count(),
        1
    );
    assert_eq!(
        history
            .iter()
            .filter(|event| matches!(&event.kind,
        EventKind::InputConsumed(data) if data.input_id == input.id))
            .count(),
        1
    );
    agent.close().await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_restore_rejects_a_newer_result_schema_instead_of_using_an_older_state() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    builder.timeline_store(store.clone());
    let agent = MinimalHost::start(&builder).await.unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    session
        .submit(
            "an older compatible result".into(),
            InputMode::Queue,
            "main",
        )
        .await
        .unwrap();
    settled("the compatible queue state is saved", session.idle("main"))
        .await
        .unwrap();
    let id = session.id().to_owned();
    agent.close().await.unwrap();
    let head = store.head(&key(&id, "main")).await.unwrap();
    append(
        &store,
        key(&id, "main"),
        vec![record(head + 1, EventKind::Extension(
        ExtensionEvent::new("input-queue", "recovered", json!({
            "schema":"input-queue/v999", "source_through":head, "state":{"future":"shape"}
        })).unwrap(),
    ))],
    )
    .await
    .unwrap();
    let agent = MinimalHost::start(&builder).await.unwrap();
    let failure = settled(
        "unknown queue schema rejects restore",
        agent.restore_session(id.clone()),
    )
    .await
    .err()
    .expect("restore must not use the older result");
    assert!(
        failure
            .to_string()
            .contains("unsupported queue recovery schema"),
        "{failure}"
    );
    assert!(agent.get_session(&id).is_none());
    let records = store
        .read(&key(&id, "main"), head + 1..=head + 1)
        .await
        .unwrap();
    assert_eq!(records[0].event.sequence, head + 1);
    agent.close().await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
}
