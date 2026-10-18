//! Session broadcast notices from their owner to attached clients (C6,
//! design/appserver/session-notices.md §9).
use super::*;
use dim_agent_capabilities::session_broadcast::{
    SessionBroadcastEvent, SessionNotice, notify, session_channel,
};
use dim_agent_runtime::{SessionIdentity, SessionScope};
use rutis::{BoxFuture, CordisError, Ctx, Effect, Listener};

/// A Session-scoped stand-in for an owner of process-level state. Like
/// SessionTitle's flow, it reacts to `input/applied` observed through the
/// broadcast and announces its own committed change with `notify`.
struct Owner {
    ctx: Ctx,
    session_id: String,
}
impl Listener<SessionBroadcastEvent> for Owner {
    fn call<'a>(
        &'a self,
        _: &'a Ctx,
        record: &'a SessionBroadcastEvent,
    ) -> BoxFuture<'a, Result<Option<()>, CordisError>> {
        if let SessionBroadcastEvent::Event { branch_id, event } = record
            && branch_id == "main"
            && matches!(event.kind, EventKind::InputApplied(_))
        {
            notify(
                &self.ctx,
                &self.session_id,
                SessionNotice {
                    source: "test.owner".into(),
                    kind: "applied".into(),
                    data: json!({"sequence": event.sequence, "nested": [1, "two", null]}),
                },
            );
        }
        Box::pin(async { Ok(None) })
    }
}
struct OwnerPlugin {
    scope: SessionScope,
}
impl TypedPlugin for OwnerPlugin {
    type Deps = InSession<SessionIdentity>;
    fn name(&self) -> &str {
        "test.notice-owner"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        InSession(identity): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            // Removed with this plugin's context.
            ctx.events().on(
                ctx,
                &session_channel(&identity.session_id),
                Owner {
                    ctx: ctx.clone(),
                    session_id: identity.session_id.clone(),
                },
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for OwnerPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

/// Every message the client received before an idle reply. The reply waits
/// for this subscription's delivery fence, which follows the turn's events and
/// any notice emitted while they were delivered.
async fn delivered(client: &mut Client, sid: &str) -> Vec<Value> {
    let idle = client
        .rpc(
            "session.idle",
            json!({"sessionId": sid, "branchId": "main"}),
        )
        .await;
    assert_ok(&idle);
    let seen = std::mem::take(&mut client.notices);
    assert!(
        seen.iter().any(|value| value["method"] == "session.event"
            && value["params"]["event"]["type"] == "turn"
            && value["params"]["event"]["name"] == "end"),
        "{seen:?}"
    );
    seen
}

/// Creates a Session, attaches every client and submits one input through
/// the first.
async fn attached_submit(clients: &mut [&mut Client]) -> String {
    let created = clients[0].rpc("session.create", create(json!([]))).await;
    let sid = created["result"]["sessionId"].as_str().unwrap().to_owned();
    for client in clients.iter_mut() {
        let attached = client
            .rpc("session.attach", json!({"sessionId": sid}))
            .await;
        assert_ok(&attached);
    }
    let input = json!({"sessionId": sid, "branchId": "main", "text": "hello"});
    assert_ok(&clients[0].rpc("session.submit", input).await);
    sid
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_owner_notice_follows_its_observed_event_to_v3_clients_only() {
    for _ in 0..5 {
        let (server, _) = launch_with(Arc::new(MockTools), ServerConfig::default(), |builder| {
            builder.session_plugin(|scope| OwnerPlugin { scope: *scope });
        })
        .await;
        let mut ipc = Client::new_version(&server, false, 3).await;
        let mut ws = Client::new_version(&server, true, 3).await;
        let mut legacy = Client::new_version(&server, false, 2).await;
        let sid = attached_submit(&mut [&mut ipc, &mut ws]).await;
        // A Session belongs to one protocol version: v2 observes its own.
        let legacy_sid = attached_submit(&mut [&mut legacy]).await;
        for client in [&mut ipc, &mut ws] {
            let seen = delivered(client, &sid).await;
            let applied = seen
                .iter()
                .position(|value| {
                    value["method"] == "session.event"
                        && value["params"]["event"]["type"] == "input"
                        && value["params"]["event"]["name"] == "applied"
                })
                .expect("input/applied reaches the client");
            let notices: Vec<_> = seen
                .iter()
                .enumerate()
                .filter(|(_, value)| value["method"] == "session.notice")
                .collect();
            assert_eq!(notices.len(), 1, "{seen:?}");
            let (position, notice) = notices[0];
            assert!(applied < position, "the notice follows its event: {seen:?}");
            assert_eq!(
                notice["params"],
                json!({
                    "sessionId": sid,
                    "source": "test.owner",
                    "kind": "applied",
                    "data": {
                        "sequence": seen[applied]["params"]["event"]["sequence"],
                        "nested": [1, "two", null],
                    },
                })
            );
        }
        // The owner announces this Session's change too; v2 receives the
        // events and no notice.
        let seen = delivered(&mut legacy, &legacy_sid).await;
        assert!(seen.iter().any(|value| value["method"] == "session.event"
            && value["params"]["event"]["name"] == "applied"));
        assert!(
            seen.iter().all(|value| value["method"] != "session.notice"),
            "{seen:?}"
        );
        server.shutdown();
        server.join().await.unwrap();
    }
}

fn notice(data: &str) -> SessionNotice {
    SessionNotice {
        source: "test.process-owner".into(),
        kind: "changed".into(),
        data: json!(data),
    }
}

/// The client's next notice, which must be `data`: any notice emitted
/// earlier on the channel would have arrived first.
async fn next_notice(client: &mut Client, sid: &str, data: &str) {
    assert_eq!(
        client.notice("session.notice").await["params"],
        json!({"sessionId": sid, "source": "test.process-owner", "kind": "changed", "data": data})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notices_without_a_listener_are_dropped_and_never_replayed() {
    let directory = std::env::temp_dir().join(format!("dim-c6-notices-{}", uuid::Uuid::new_v4()));
    let (server, agent) = launch_with_agent(
        Arc::new(MockTools),
        ServerConfig {
            database_path: Some(directory.join("dimcode.sqlite")),
            ..Default::default()
        },
    )
    .await;
    for _ in 0..3 {
        for transport in [false, true] {
            let mut client = Client::new_version(&server, transport, 3).await;
            let created = client.rpc("session.create", create(json!([]))).await;
            let sid = created["result"]["sessionId"].as_str().unwrap().to_owned();
            let attach = json!({"sessionId": sid});
            assert_ok(&client.rpc("session.attach", attach.clone()).await);
            server.notify(&sid, notice("attached"));
            next_notice(&mut client, &sid, "attached").await;

            // Loaded, but nobody observes the Session.
            assert_ok(&client.rpc("session.detach", attach.clone()).await);
            server.notify(&sid, notice("detached"));
            assert_ok(&client.rpc("session.attach", attach.clone()).await);
            server.notify(&sid, notice("reattached"));
            next_notice(&mut client, &sid, "reattached").await;

            // Stored but not loaded; the attach restores it.
            assert_ok(&client.rpc("session.detach", attach.clone()).await);
            super::collaboration::unload(&agent, &sid).await;
            server.notify(&sid, notice("unloaded"));
            assert_ok(&client.rpc("session.attach", attach.clone()).await);
            server.notify(&sid, notice("restored"));
            next_notice(&mut client, &sid, "restored").await;

            // The close reply follows `Closed`, so this notice follows it on
            // the channel; the subscription ends with `session.closed`.
            assert_ok(&client.rpc("session.close", attach).await);
            server.notify(&sid, notice("closed"));
            client.notice("session.closed").await;
            assert!(
                client
                    .notices
                    .iter()
                    .all(|value| value["method"] != "session.notice"),
                "{:?}",
                client.notices
            );
        }
    }
    server.shutdown();
    server.join().await.unwrap();
    let _ = std::fs::remove_dir_all(directory);
}
