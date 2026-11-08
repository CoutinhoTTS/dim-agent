mod support;

use dim_agent_types::*;
use dim_app_server::storage::{SqliteStorage, SqliteTimelineStore, sqlite_memory};
use support::{parts::*, texts};

#[path = "../../../xtask/src/allocation.rs"]
mod allocation;
#[global_allocator]
static ALLOCATOR: allocation::MeasuredAllocator = allocation::MeasuredAllocator;

#[tokio::test(flavor = "current_thread")]
#[ignore = "resource measurement over disposable SQLite stores"]
async fn sqlite_goal_restore_cost() {
    let sizes = std::env::var("DIM_GOAL_QUERY_INPUTS").map_or_else(
        |_| vec![100, 1_000, 5_000],
        |sizes| {
            sizes
                .split(',')
                .map(|size| {
                    size.parse()
                        .expect("DIM_GOAL_QUERY_INPUTS contains positive integers")
                })
                .collect()
        },
    );
    for n in sizes {
        let path =
            std::env::temp_dir().join(format!("dim-goal-query-{}.sqlite", uuid::Uuid::new_v4()));
        let db = SqliteStorage::open(path.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        let env = Env::start_store(store.clone()).await;
        let mut seed = env.seed_new("goal-query-cost").await;
        seed.append(
            "main",
            EventKind::BranchStart(BranchStartData {
                parent: None,
                create_event: None,
            }),
        );
        let (objective, status) = texts::legacy_set_blocks("main", "ship it");
        let set = seed.append(
            "main",
            message(
                vec![
                    raw(&objective, Some("goal:objective"), false),
                    raw(&status, Some("goal:status"), false),
                ],
                InputMode::Queue,
            ),
        );
        for _ in 0..n {
            let input = seed.append(
                "main",
                user_with_blocks(
                    &"u".repeat(4096),
                    InputMode::Queue,
                    vec![raw(&"b".repeat(4096), None, false)],
                ),
            );
            seed.append(
                "main",
                EventKind::InputConsumed(InputConsumedData {
                    input_id: input.id.clone(),
                }),
            );
        }
        seed.append(
            "main",
            EventKind::TurnStart(TurnStartData {
                turn_id: "work".into(),
            }),
        );
        seed.append(
            "main",
            EventKind::InputConsumed(InputConsumedData {
                input_id: set.id.clone(),
            }),
        );
        let end = seed.append("main", turn_end_with("work", "completed"));
        seed.finish().await;
        let end_sequence = end.sequence;
        store.record_query_statistics();
        let _ = store.take_node_loads();
        let (sqlite_baseline, _) = sqlite_memory(true);
        let baseline = allocation::live();
        allocation::reset_peak_now();
        let now = std::time::Instant::now();
        let env = env.restore_seeded("goal-query-cost").await;
        let statements = store.take_query_statistics();
        let (sqlite_after, sqlite_peak) = sqlite_memory(false);
        println!(
            "{}",
            serde_json::json!({"n": n, "events":end_sequence, "elapsedMs": now.elapsed().as_secs_f64()*1000., "rustPeakAdditionalBytes": allocation::peak().saturating_sub(baseline), "rustRetainedBytes": allocation::live() as i64-baseline as i64, "nodeLoads": store.take_node_loads().loads, "sqliteBeforeBytes": sqlite_baseline, "sqliteAfterBytes": sqlite_after, "sqlitePeakBytes": sqlite_peak, "sqlitePeakAdditionalBytes": sqlite_peak - sqlite_baseline, "sqliteRetainedBytes": sqlite_after - sqlite_baseline, "statements": statements})
        );
        let continuation = env.wait_for("main", r#"kind="continuation""#).await;
        assert_eq!(
            continuation.text,
            texts::continuation("main", 1, None, 0, end_sequence)
        );
        env.close().await;
        store.close().await.unwrap();
        db.close().await.unwrap();
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
