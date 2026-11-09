use super::*;
use dim_agent_runtime::{
    QueryComparison, QueryEvent, QueryFilter, QueryOperand, QueryOrder, QueryOrderKey, QueryOutput,
    QueryPage, QuerySelect, QuerySpec, TimelineQuery, TimelineQueryDirection, TimelineQueryReply,
};
use serde_json::{Value, json};
use std::num::NonZeroU64;

pub(super) fn query(scope: TimelineQueryScope, limit: u64) -> TimelineQuery {
    TimelineQuery {
        scope,
        from: None,
        through: None,
        include_publishing: false,
        query: QuerySpec {
            rows: None,
            filter: None,
            select: QuerySelect::Events,
            group_by: vec![],
            pick: None,
            order_by: vec![QueryOrder {
                key: QueryOrderKey::Position,
                direction: TimelineQueryDirection::Ascending,
            }],
        },
        page: Some(QueryPage {
            limit: NonZeroU64::new(limit).unwrap(),
            after: None,
        }),
    }
}

pub(super) fn eq(path: &str, value: Value) -> QueryFilter {
    QueryFilter::Compare {
        left: QueryOperand::Field { path: path.into() },
        comparison: QueryComparison::Eq,
        right: QueryOperand::Value { value },
    }
}

fn query_ids(reply: &TimelineQueryReply) -> Vec<String> {
    let QueryOutput::Events { events } = &reply.result else {
        panic!("events")
    };
    events.iter().map(|row| row.event.id.clone()).collect()
}

async fn pages(timeline: &Timeline, mut request: TimelineQuery) -> Vec<QueryEvent> {
    let mut result = Vec::new();
    loop {
        let reply = timeline.query(request.clone()).await.unwrap();
        let QueryOutput::Events { events } = reply.result else {
            panic!("events")
        };
        assert!(events.len() <= request.page.as_ref().unwrap().limit.get() as usize);
        if reply.next.is_some() {
            assert!(!events.is_empty(), "a continuing page makes progress");
        }
        result.extend(events);
        request.through = Some(reply.through);
        let Some(next) = reply.next else {
            return result;
        };
        request.page.as_mut().unwrap().after = Some(next);
    }
}

#[tokio::test]
async fn property_selections_and_keyset_pages_agree_across_storage_backends() {
    use TimelineQueryDirection::{Ascending, Descending};
    use TimelineQueryScope::{Branch, Chain};
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let (sqlite_main, sqlite_child) = query_histories(store.clone()).await;
    let (memory_main, memory_child) =
        query_histories(Arc::new(MemoryTimelineStore::default())).await;
    let and = |conditions| QueryFilter::And { conditions };
    let or = |conditions| QueryFilter::Or { conditions };
    let cases = vec![
        (
            Branch,
            None,
            vec!["main-1", "main-2", "main-3", "main-4", "main-5", "main-6"],
        ),
        (
            Branch,
            Some(eq("kind", json!("step/text"))),
            vec!["main-4", "main-5"],
        ),
        (Branch, Some(eq("kind", json!("step/tex"))), vec![]),
        (
            Branch,
            Some(eq("context.render", json!(true))),
            vec!["main-4", "main-5"],
        ),
        (
            Branch,
            Some(eq("context.state", json!(true))),
            vec!["main-1", "main-4", "main-5"],
        ),
        (
            Branch,
            Some(or(vec![
                eq("context.render", json!(true)),
                eq("context.state", json!(true)),
            ])),
            vec!["main-1", "main-4", "main-5"],
        ),
        (
            Branch,
            Some(eq("data.tool", json!("search"))),
            vec!["main-2"],
        ),
        (
            Branch,
            Some(eq("data.nested.level", json!(3))),
            vec!["main-2"],
        ),
        (Branch, Some(eq("data.nested.absent", json!(3))), vec![]),
        (Branch, Some(eq("data.delta", json!("b"))), vec!["main-5"]),
        (
            Branch,
            Some(eq("data.turn_id", json!("turn"))),
            vec!["main-4", "main-5"],
        ),
        (
            Branch,
            Some(eq("data.request_id", json!("request"))),
            vec!["main-4", "main-5"],
        ),
        (Branch, Some(eq("data.request_id", json!("absent"))), vec![]),
        (Branch, Some(eq("id", json!("main-2"))), vec!["main-2"]),
        (Branch, Some(eq("id", json!("absent"))), vec![]),
        (
            Branch,
            Some(and(vec![
                eq("id", json!("main-2")),
                eq("kind", json!("step/text")),
            ])),
            vec![],
        ),
        (
            Branch,
            Some(and(vec![
                eq("id", json!("main-1")),
                eq("context.render", json!(true)),
            ])),
            vec![],
        ),
        (
            Branch,
            Some(and(vec![
                eq("id", json!("main-4")),
                eq("data.delta", json!("a")),
            ])),
            vec!["main-4"],
        ),
        (
            Branch,
            Some(and(vec![
                eq("id", json!("main-4")),
                eq("data.delta", json!("b")),
            ])),
            vec![],
        ),
        (
            Chain,
            None,
            vec!["main-1", "main-2", "child-1", "child-2", "child-3"],
        ),
        (
            Chain,
            Some(eq("data.tool", json!("search"))),
            vec!["main-2", "child-1"],
        ),
        (Chain, Some(eq("id", json!("main-4"))), vec![]),
        (Chain, Some(eq("id", json!("child-2"))), vec!["child-2"]),
        (
            Chain,
            Some(eq("kind", json!("input/message"))),
            vec!["main-1", "child-2", "child-3"],
        ),
    ];
    for (scope, filter, expected) in cases {
        let (sqlite, memory) = if scope == Branch {
            (&sqlite_main, &memory_main)
        } else {
            (&sqlite_child, &memory_child)
        };
        for direction in [Ascending, Descending] {
            for limit in [1, 2, 16] {
                let mut request = query(scope, limit);
                request.query.filter = filter.clone();
                request.query.order_by[0].direction = direction;
                let left = pages(sqlite, request.clone()).await;
                let right = pages(memory, request.clone()).await;
                let mut expected = expected.clone();
                if direction == Descending {
                    expected.reverse();
                }
                assert_eq!(
                    left.iter()
                        .map(|row| row.event.id.as_str())
                        .collect::<Vec<_>>(),
                    expected,
                    "{request:?}"
                );
                assert_eq!(
                    serde_json::to_value(&left).unwrap(),
                    serde_json::to_value(&right).unwrap(),
                    "{request:?}"
                );
            }
        }
    }
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn property_pages_keep_explicit_bounds_inside_merged_output() {
    use TimelineQueryDirection::{Ascending, Descending};
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let (sqlite, _) = query_histories(store.clone()).await;
    let (memory, _) = query_histories(Arc::new(MemoryTimelineStore::default())).await;
    for timeline in [&sqlite, &memory] {
        for direction in [Ascending, Descending] {
            let mut request = query(TimelineQueryScope::Branch, 1);
            request.query.order_by[0].direction = direction;
            request.through = Some(4);
            request.query.filter = Some(eq("kind", json!("step/text")));
            let reply = timeline.query(request.clone()).await.unwrap();
            assert_eq!(query_ids(&reply), ["main-4"]);
            assert!(
                reply.next.is_none(),
                "the later delta is outside the fixed cut"
            );
            request.query.filter = Some(eq("data.delta", json!("b")));
            assert!(query_ids(&timeline.query(request).await.unwrap()).is_empty());
        }
        let mut request = query(TimelineQueryScope::Branch, 1);
        request.from = Some(TimelinePosition {
            branch_id: "main".into(),
            sequence: 5,
        });
        let rows = pages(timeline, request).await;
        assert_eq!(
            rows.iter()
                .map(|row| row.event.id.as_str())
                .collect::<Vec<_>>(),
            ["main-5", "main-6"]
        );
    }
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn property_pages_cross_tall_ancestors_and_empty_branches() {
    use TimelineQueryDirection::{Ascending, Descending};
    for child_facts in [0, 3] {
        let database = Database::new();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        let (_, sqlite) = tall_chain_histories(store.clone(), child_facts).await;
        let (_, memory) =
            tall_chain_histories(Arc::new(MemoryTimelineStore::default()), child_facts).await;
        for direction in [Ascending, Descending] {
            let mut request = query(TimelineQueryScope::Chain, 3);
            request.query.order_by[0].direction = direction;
            let left = pages(&sqlite, request.clone()).await;
            let right = pages(&memory, request).await;
            let mut expected: Vec<_> = (1..=5)
                .map(|n| ("main", n))
                .chain((1..=child_facts).map(|n| ("child", n)))
                .collect();
            if direction == Descending {
                expected.reverse();
            }
            assert_eq!(
                left.iter()
                    .map(|row| (row.position.branch_id.as_str(), row.position.sequence))
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                serde_json::to_value(&left).unwrap(),
                serde_json::to_value(&right).unwrap()
            );
        }
        store.close().await.unwrap();
        db.close().await.unwrap();
    }
}

#[tokio::test]
async fn property_pages_order_interleaved_kinds_by_position() {
    use TimelineQueryDirection::{Ascending, Descending};
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let sqlite = interleaved_histories(store.clone()).await;
    let memory = interleaved_histories(Arc::new(MemoryTimelineStore::default())).await;
    for direction in [Ascending, Descending] {
        let mut request = query(TimelineQueryScope::Branch, 1);
        request.query.order_by[0].direction = direction;
        request.query.filter = Some(QueryFilter::In {
            value: QueryOperand::Field {
                path: "kind".into(),
            },
            values: vec![json!("turn/start"), json!("step/model-start")],
        });
        let left = pages(&sqlite, request.clone()).await;
        let right = pages(&memory, request).await;
        let mut expected = vec!["mixed-1", "mixed-2", "mixed-3", "mixed-4"];
        if direction == Descending {
            expected.reverse();
        }
        assert_eq!(
            left.iter()
                .map(|row| row.event.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            serde_json::to_value(&left).unwrap(),
            serde_json::to_value(&right).unwrap()
        );
    }
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn property_pages_span_large_batches_and_sparse_merged_matches() {
    use TimelineQueryDirection::{Ascending, Descending};
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let sqlite_store = SqliteTimelineStore::new(db.clone());
    let sqlite: Arc<dyn TimelineStore> = sqlite_store.clone();
    let memory: Arc<dyn TimelineStore> = Arc::new(MemoryTimelineStore::default());
    for store in [&sqlite, &memory] {
        store
            .create_session("wide", &SessionConfig::default())
            .await
            .unwrap();
        for batch in 0..3 {
            let records = (batch * 100 + 1..=(batch + 1) * 100)
                .map(|n| {
                    query_record(
                        n,
                        (batch + 1) * 100,
                        &format!("wide-{n}"),
                        delta(if n % 100 == 0 { "hit" } else { "plain" }),
                    )
                })
                .collect();
            publish_records(store, key("wide", "main"), records).await;
        }
        let timeline = Timeline::restore(
            rutis::Ctx::root().unwrap(),
            key("wide", "main"),
            vec![],
            store.clone(),
        )
        .await
        .unwrap();
        for direction in [Ascending, Descending] {
            for limit in [1, 200] {
                let mut request = query(TimelineQueryScope::Branch, limit);
                request.query.order_by[0].direction = direction;
                let all = pages(&timeline, request.clone()).await;
                let mut expected: Vec<_> = (1..=300).collect();
                if direction == Descending {
                    expected.reverse();
                }
                assert_eq!(
                    all.iter()
                        .map(|row| row.position.sequence)
                        .collect::<Vec<_>>(),
                    expected
                );
                request.query.filter = Some(eq("data.delta", json!("hit")));
                let hits = pages(&timeline, request).await;
                expected.retain(|n| n % 100 == 0);
                assert_eq!(
                    hits.iter()
                        .map(|row| row.position.sequence)
                        .collect::<Vec<_>>(),
                    expected
                );
            }
        }
        let mut request = query(TimelineQueryScope::Branch, 10);
        request.from = Some(TimelinePosition {
            branch_id: "main".into(),
            sequence: 251,
        });
        let reply = timeline.query(request).await.unwrap();
        assert_eq!(
            query_ids(&reply),
            (251..=260).map(|n| format!("wide-{n}")).collect::<Vec<_>>()
        );
    }
    sqlite_store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn a_query_compares_numbers_by_value_without_coercing_booleans() {
    use dim_agent_runtime::TimelineQueryDirection::{Ascending, Descending};
    use dim_agent_runtime::TimelineQueryScope::Branch;

    let values = |sequence: u64| {
        [
            (1u64, serde_json::json!(true)),
            (2, serde_json::json!(1)),
            (3, serde_json::json!(1.0)),
        ]
        .into_iter()
        .find(|(at, _)| *at == sequence)
        .map(|(_, value)| value)
        .unwrap()
    };
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store: std::sync::Arc<dyn TimelineStore> = SqliteTimelineStore::new(db.clone());
    let memory_store: std::sync::Arc<dyn TimelineStore> =
        std::sync::Arc::new(dim_agent_runtime::MemoryTimelineStore::default());
    for target in [store.clone(), memory_store.clone()] {
        target
            .create_session("query", &SessionConfig::default())
            .await
            .unwrap();
        for sequence in 1..=3u64 {
            publish_records(
                &target,
                key("query", "main"),
                vec![query_record(
                    sequence,
                    sequence,
                    &format!("value-{sequence}"),
                    marked(serde_json::json!({"value": values(sequence)})),
                )],
            )
            .await;
        }
    }
    let sqlite = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("query", "main"),
        vec![],
        store.clone(),
    )
    .await
    .unwrap();
    let memory = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("query", "main"),
        vec![],
        memory_store,
    )
    .await
    .unwrap();

    for (direction, value, expected) in [
        (Ascending, serde_json::json!(1), "value-2"),
        (Ascending, serde_json::json!(1.0), "value-2"),
        (Descending, serde_json::json!(1), "value-3"),
        (Descending, serde_json::json!(1.0), "value-3"),
        (Descending, serde_json::json!(true), "value-1"),
    ] {
        let mut request = query(Branch, 1);
        request.query.order_by[0].direction = direction;
        request.query.filter = Some(eq("data.value", value));
        let from_sqlite = sqlite.query(request.clone()).await.unwrap();
        let from_memory = memory.query(request).await.unwrap();
        assert_eq!(query_ids(&from_sqlite), [expected], "{direction:?} sqlite");
        assert_eq!(query_ids(&from_memory), [expected], "{direction:?} memory");
    }
    db.close().await.unwrap();
}

#[tokio::test]
async fn a_query_compares_a_large_integer_exactly() {
    use dim_agent_runtime::TimelineQueryScope::Branch;

    let small = serde_json::json!(18_446_744_073_709_551_614u64);
    let large = serde_json::json!(18_446_744_073_709_551_615u64);
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store: std::sync::Arc<dyn TimelineStore> = SqliteTimelineStore::new(db.clone());
    let memory_store: std::sync::Arc<dyn TimelineStore> =
        std::sync::Arc::new(dim_agent_runtime::MemoryTimelineStore::default());
    for target in [store.clone(), memory_store.clone()] {
        target
            .create_session("query", &SessionConfig::default())
            .await
            .unwrap();
        for (sequence, value) in [(1u64, small.clone()), (2, large.clone())] {
            publish_records(
                &target,
                key("query", "main"),
                vec![query_record(
                    sequence,
                    sequence,
                    &format!("value-{sequence}"),
                    marked(serde_json::json!({"value": value})),
                )],
            )
            .await;
        }
    }
    let sqlite = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("query", "main"),
        vec![],
        store.clone(),
    )
    .await
    .unwrap();
    let memory = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("query", "main"),
        vec![],
        memory_store,
    )
    .await
    .unwrap();

    let mut request = query(Branch, 1);
    request.query.filter = Some(eq("data.value", large));
    let from_sqlite = sqlite.query(request.clone()).await.unwrap();
    let from_memory = memory.query(request).await.unwrap();
    assert_eq!(query_ids(&from_sqlite), ["value-2"], "sqlite");
    assert_eq!(query_ids(&from_memory), ["value-2"], "memory agrees");
    db.close().await.unwrap();
}

#[tokio::test]
async fn character_totals_agree_for_unicode_nul_and_missing_properties() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let stores: Vec<Arc<dyn TimelineStore>> = vec![
        SqliteTimelineStore::new(db.clone()),
        Arc::new(MemoryTimelineStore::default()),
    ];
    for store in stores {
        store
            .create_session("characters", &SessionConfig::default())
            .await
            .unwrap();
        let key = key("characters", "main");
        publish_records(
            &store,
            key.clone(),
            vec![
                query_record(1, 1, "a", delta("你好\0")),
                query_record(2, 2, "b", delta("é🦀")),
                query_record(3, 3, "c", marked(json!({"delta":null}))),
                query_record(4, 4, "d", marked(json!({"delta":7}))),
                query_record(5, 5, "e", marked(json!({}))),
            ],
        )
        .await;
        let timeline = Timeline::restore(rutis::Ctx::root().unwrap(), key, vec![], store.clone())
            .await
            .unwrap();
        let mut request = query(TimelineQueryScope::Branch, 10);
        request.query.select = QuerySelect::Characters {
            field: "data.delta".into(),
        };
        request.query.order_by.clear();
        request.page = None;
        let reply = timeline.query(request.clone()).await.unwrap();
        assert!(matches!(reply.result, QueryOutput::Count { count: 5 }));
        request.through = Some(1);
        assert!(matches!(
            timeline.query(request).await.unwrap().result,
            QueryOutput::Count { count: 3 }
        ));
        drop(timeline);
    }
    db.close().await.unwrap();
}
