use super::*;
use dim_agent_runtime::BranchOptions;
use dim_agent_types::{BranchCreateAt, BranchSource, EventKind, InputMode};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_transports_encode_the_v1_branch_create_boundary_live() {
    for ws in [false, true] {
        let (server, agent) = launch_with_agent(Arc::new(MockTools), ServerConfig::default()).await;
        let mut live = Client::new(&server, ws).await;
        let created = live.rpc("session.create", create(json!(["echo"]))).await;
        assert_ok(&created);
        let id = created["result"]["sessionId"].as_str().unwrap();
        let session = agent.get_session(id).unwrap();
        session
            .submit("parent".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
        session.stop("main", "control cursor".into()).await.unwrap();
        let high = session.sequence("main").unwrap();
        let initial = live.rpc("session.attach", json!({"sessionId":id})).await;
        assert_eq!(initial["result"], json!({}));
        let child = session
            .branch(
                "main".into(),
                high,
                BranchOptions {
                    prompt: "child".into(),
                    source: BranchSource::Tool {
                        turn_id: "turn".into(),
                        request_id: "request".into(),
                        call_id: "call".into(),
                    },
                },
            )
            .await
            .unwrap();
        session.idle(&child).await.unwrap();
        let events = session.local("main", ..).await.unwrap();
        let create = events.last().unwrap();
        assert!(
            matches!(&create.kind, EventKind::BranchCreate(data) if data.at == BranchCreateAt::Create {} && data.branch == child)
        );
        let notices = receive_events_through(&mut live, create.sequence).await;
        assert_eq!(notices, v1_events(&events, high));
        assert!(notices[0]["data"].get("at").is_none());
        assert_eq!(notices[0]["sequence"], create.sequence);
        let local = session.local(&child, ..).await.unwrap();
        assert!(
            matches!(&local[0].kind, EventKind::BranchStart(data) if data.create_event == Some(create.sequence))
        );
        server.shutdown();
        server.join().await.unwrap();
    }
}
