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
async fn sqlite_goal_driving_write_and_recovery_cost() {
    for n in [100, 1_000] {
        let path =
            std::env::temp_dir().join(format!("dim-goal-write-{}.sqlite", uuid::Uuid::new_v4()));
        let db = SqliteStorage::open(path.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        let env = Env::start_store(store.clone()).await;
        env.set("ship it").await;
        let baseline = allocation::live();
        allocation::reset_peak_now();
        let now = std::time::Instant::now();
        for index in 0..n {
            env.emit("main", user(&format!("pending {index}"), InputMode::Queue));
        }
        env.goal("get", serde_json::json!({})).await.unwrap();
        let elapsed = now.elapsed();
        let peak = allocation::peak().saturating_sub(baseline);
        let mut nodes = 0;
        let mut results = 0;
        let mut bytes = 0;
        let mut max_result_bytes = 0;
        for event in env.all_events("main").await {
            if let EventKind::Extension(data) = &event.kind {
                let size = serde_json::to_vec(data.data()).unwrap().len();
                match data.key() {
                    "goal/driving-node" => {
                        nodes += 1;
                        bytes += size;
                    }
                    "goal/recovered" => {
                        results += 1;
                        bytes += size;
                        max_result_bytes = max_result_bytes.max(size);
                        assert!(data.data().pointer("/state/fold/driving").is_none());
                    }
                    _ => {}
                }
            }
        }
        let files = [
            path.clone(),
            std::path::PathBuf::from(format!("{}-wal", path.display())),
        ];
        let retained_file_bytes: u64 = files
            .iter()
            .filter_map(|file| std::fs::metadata(file).ok())
            .map(|metadata| metadata.len())
            .sum();
        store.record_query_statistics();
        let now = std::time::Instant::now();
        let env = env.restore().await;
        assert_eq!(
            env.goal("get", serde_json::json!({})).await.unwrap()["goal"]["objective"],
            "ship it"
        );
        let vm_steps: i64 = store
            .take_query_statistics()
            .iter()
            .map(|stat| i64::from(stat.vm_steps))
            .sum();
        println!(
            "{}",
            serde_json::json!({"phase":"driving-writes", "n":n,
            "writeElapsedMs":elapsed.as_secs_f64()*1000., "writePeakAdditionalBytes":peak,
            "nodeFacts":nodes, "resultFacts":results,"factPayloadBytes":bytes,
            "maxResultBytes":max_result_bytes,"retainedDbAndWalBytes":retained_file_bytes,
            "restoreElapsedMs":now.elapsed().as_secs_f64()*1000.,"restoreVmSteps":vm_steps})
        );
        env.close().await;
        store.close().await.unwrap();
        db.close().await.unwrap();
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

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
        env.goal("get", serde_json::json!({})).await.unwrap();
        let _ = store.take_query_statistics();
        let _ = store.take_node_loads();
        store.record_query_statistics();
        let now = std::time::Instant::now();
        let env = env.restore().await;
        let reply = env.goal("get", serde_json::json!({})).await.unwrap();
        assert_eq!(reply["goal"]["objective"], "ship it");
        let statements = store.take_query_statistics();
        assert!(
            !statements.is_empty(),
            "cold recovery queries were measured"
        );
        let vm_steps: i64 = statements
            .iter()
            .map(|statement| i64::from(statement.vm_steps))
            .sum();
        let loads = store.take_node_loads();
        println!(
            "{}",
            serde_json::json!({"phase":"saved-result-resume", "n":n,
                "elapsedMs":now.elapsed().as_secs_f64()*1000., "vmSteps":vm_steps,
                "nodeLoads":loads.loads, "encodedNodes":loads.encoded, "statements":statements})
        );
        let now = std::time::Instant::now();
        store.record_query_statistics();
        env.goal("get", serde_json::json!({})).await.unwrap();
        assert!(
            store.take_query_statistics().is_empty(),
            "a live Goal get does not read SQLite"
        );
        assert_eq!(
            store.take_node_loads().loads,
            0,
            "a live get does not load content nodes"
        );
        println!(
            "{}",
            serde_json::json!({"phase":"live-get", "n":n,
            "elapsedMs":now.elapsed().as_secs_f64()*1000., "vmSteps":0, "nodeLoads":0})
        );
        env.close().await;
        store.close().await.unwrap();
        db.close().await.unwrap();
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
