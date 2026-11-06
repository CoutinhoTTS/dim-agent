//! Connection-bound notifications (methods contract A.2): `callChannel`
//! `notify` and `closed` as the Rust binding delivers them, through both
//! method tables, with ordering, refusal and lifetime.
use super::*;
use dim_agent_capabilities::methods::{
    Caller, Interruption, MethodCall, MethodDecl, NotifyError, RpcError, SessionMethods,
};
use dim_agent_runtime::Feature;
use dim_app_server::methods::ManagementMethods;
use rutis::host::{BoxFuture, DynPlugin, Effect, Key, TypeKey};
use serde_json::Map;

/// What the provider of a test records between calls.
#[derive(Default)]
struct ChannelState {
    /// The caller of the last call that kept one.
    caller: std::sync::Mutex<Option<Caller>>,
    /// A held caller's `closed` resolved.
    closed: Arc<tokio::sync::Notify>,
    /// Releases `channel.stream`'s task, which notifies after the reply.
    release: Arc<tokio::sync::Notify>,
    /// `channel.fill` finished: the first code the full allowance returned
    /// and the code of one more notification after it.
    filled: std::sync::Mutex<Option<(&'static str, &'static str)>>,
}
impl ChannelState {
    fn caller(&self) -> Caller {
        self.caller
            .lock()
            .unwrap()
            .clone()
            .expect("a call kept its caller")
    }
    fn code(&self, method: &str, params: Map<String, Value>) -> Value {
        match self.caller().notify(method, params) {
            Ok(()) => json!({"ok": true}),
            Err(error) => json!({"code": error.code()}),
        }
    }
}

/// A declaration with the notifications it declares; a notification belongs
/// to the provider generation, so each name is declared once.
fn notification(name: &str, notifications: &[&str]) -> MethodDecl {
    MethodDecl {
        notifications: notifications.iter().map(|name| (*name).into()).collect(),
        ..MethodDecl::new(name, false, Interruption::Drain)
    }
}

/// The handler both test providers serve: `stream` sends one notification
/// inside the call and one from a task after it, `start` keeps the caller,
/// and `attempt` sends through the kept one.
fn serve(call: MethodCall, state: Arc<ChannelState>) -> Result<Value, RpcError> {
    let caller = call
        .origin
        .caller
        .ok_or_else(|| RpcError::wire(-32603, "no caller"))?;
    let session = call.method.starts_with("session.");
    let names = match session {
        true => ("session.channel.first", "session.channel.second"),
        false => ("channel.first", "channel.second"),
    };
    match call.method.as_str() {
        "channel.stream" | "session.channel.stream" => {
            *state.caller.lock().unwrap() = Some(caller.clone());
            let first = caller.notify(names.0, map(json!({"step": 1})));
            let release = state.release.clone();
            tokio::spawn(async move {
                release.notified().await;
                let _ = caller.notify(names.1, map(json!({"step": 2})));
            });
            Ok(match first {
                Ok(()) => json!({"ok": true}),
                Err(error) => json!({"code": error.code()}),
            })
        }
        "channel.start" => {
            *state.caller.lock().unwrap() = Some(caller.clone());
            if call.params["hold"] == true {
                let closed = caller.closed();
                let state = state.clone();
                tokio::spawn(async move {
                    closed.await;
                    state.closed.notify_one();
                });
            }
            Ok(json!({}))
        }
        "channel.fill" => {
            let count = call.params["count"].as_u64().unwrap_or(0) as usize;
            let mut first = "ok";
            for step in 0..count {
                if let Err(error) = caller.notify(names.0, map(json!({"step": step}))) {
                    first = error.code();
                    break;
                }
            }
            let later = match caller.notify(names.0, map(json!({"late": true}))) {
                Ok(()) => "ok",
                Err(error) => error.code(),
            };
            *state.filled.lock().unwrap() = Some((first, later));
            Ok(json!({}))
        }
        "channel.attempt" => {
            let name = call.params["method"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let bytes = call.params["bytes"].as_u64().unwrap_or(0) as usize;
            let mut params = Map::new();
            if bytes > 0 {
                params.insert("text".into(), json!("x".repeat(bytes)));
            }
            Ok(state.code(&name, params))
        }
        _ => Err(RpcError::unknown_method()),
    }
}

fn map(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// Processes the management namespace `channel`.
struct ChannelMethods {
    table: Key<ManagementMethods>,
    state: Arc<ChannelState>,
}
impl DynPlugin for ChannelMethods {
    fn name(&self) -> &str {
        "channel-methods-fixture"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.table.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let state = self.state.clone();
            ctx.require_at(&self.table)?.register(
                ctx,
                "channel",
                vec![
                    notification(
                        "channel.stream",
                        &["channel.first", "channel.second", "channel.other"],
                    ),
                    notification("channel.start", &[]),
                    notification("channel.attempt", &[]),
                    notification("channel.fill", &[]),
                ],
                Arc::new(move |call, _, _| {
                    let state = state.clone();
                    Box::pin(async move { serve(call, state) })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}

/// Serves the Session namespace `session.channel`.
struct ChannelSession {
    methods: Key<SessionMethods>,
    state: Arc<ChannelState>,
}
impl DynPlugin for ChannelSession {
    fn name(&self) -> &str {
        "channel-session-fixture"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.methods.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let state = self.state.clone();
            ctx.require_at(&self.methods)?.register(
                ctx,
                "session.channel",
                vec![notification(
                    "session.channel.stream",
                    &["session.channel.first", "session.channel.second"],
                )],
                Arc::new(move |call, _, _| {
                    let state = state.clone();
                    Box::pin(async move { serve(call, state) })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}

fn channel_plugin(
    state: Arc<ChannelState>,
) -> impl FnOnce(&mut AgentBuilder) + Send + Sync + 'static {
    move |builder| {
        let management = state.clone();
        builder.plugin(move |names| {
            Ok(Box::new(ChannelMethods {
                table: names.key()?,
                state: management.clone(),
            }) as Box<dyn DynPlugin>)
        });
        builder.require_feature(Feature::new("ChannelSession").session(
            "ChannelSession",
            move |names| {
                Ok(Box::new(ChannelSession {
                    methods: names.key()?,
                    state: state.clone(),
                }))
            },
        ));
    }
}

/// Launches with the channel providers and returns one v3 client.
async fn launch_channel() -> (RunningServer, Client, Arc<ChannelState>) {
    let state = Arc::new(ChannelState::default());
    let (server, _) = launch_with(
        Arc::new(MockTools),
        ServerConfig::default(),
        channel_plugin(state.clone()),
    )
    .await;
    let client = Client::new_version(&server, false, 3).await;
    (server, client, state)
}

/// A.2: a notification sent while the call runs is held and written right
/// after its reply; one sent later follows in order. Only the calling
/// connection receives them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notifications_follow_the_reply_in_order_on_the_calling_connection_only() {
    let (server, mut client, state) = launch_channel().await;
    let mut other = Client::new_version(&server, false, 3).await;
    let id = client.send("channel.stream", json!({})).await;
    let reply = client.read("the reply").await;
    assert_eq!(reply["id"], id, "{reply}");
    assert_eq!(reply["result"], json!({"ok": true}));
    let first = client.read("the first notification").await;
    assert_eq!(first["jsonrpc"], "2.0");
    assert_eq!(first["method"], "channel.first");
    assert_eq!(first["params"], json!({"step": 1}));
    state.release.notify_one();
    let second = client.read("the second notification").await;
    assert_eq!(second["method"], "channel.second");
    assert_eq!(second["params"], json!({"step": 2}));
    assert_ok(&other.rpc("daemon.ping", json!({})).await);
    assert!(
        other.notices.is_empty(),
        "another connection received the notifications: {:?}",
        other.notices
    );
    server.shutdown();
    server.join().await.unwrap();
}

/// A.2: a name the provider generation did not declare is `Undeclared` and
/// nothing is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_undeclared_notification_name_writes_nothing() {
    let (server, mut client, _) = launch_channel().await;
    assert_ok(&client.rpc("channel.start", json!({})).await);
    let reply = client
        .rpc("channel.attempt", json!({"method": "channel.undeclared"}))
        .await;
    assert_eq!(reply["result"], json!({"code": "undeclared"}), "{reply}");
    assert_ok(&client.rpc("daemon.ping", json!({})).await);
    assert!(
        client.notices.is_empty(),
        "an undeclared name was written: {:?}",
        client.notices
    );
    server.shutdown();
    server.join().await.unwrap();
}

/// A.2 acceptance 6: a caller's disconnect resolves `closed()` and later
/// `notify` calls answer `Gone`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disconnect_resolves_closed_and_later_notifications_answer_gone() {
    let (server, mut client, state) = launch_channel().await;
    assert_ok(&client.rpc("channel.start", json!({"hold": true})).await);
    drop(client);
    tokio::time::timeout(std::time::Duration::from_secs(10), state.closed.notified())
        .await
        .expect("the caller's closed did not resolve");
    let mut other = Client::new_version(&server, false, 3).await;
    let reply = other
        .rpc("channel.attempt", json!({"method": "channel.first"}))
        .await;
    assert_eq!(reply["result"], json!({"code": "gone"}), "{reply}");
    server.shutdown();
    server.join().await.unwrap();
}

/// A.2: an Rpc reload resolves `closed()` and later `notify` calls answer
/// `Gone` (conflict table, "Rpc reload with calls in flight").
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_rpc_reload_resolves_closed_and_later_notifications_answer_gone() {
    let (server, mut client, state) = launch_channel().await;
    assert_ok(&client.rpc("channel.start", json!({"hold": true})).await);
    server.restart("Rpc").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), state.closed.notified())
        .await
        .expect("the caller's closed did not resolve");
    let mut other = Client::new_version(&server, false, 3).await;
    let reply = other
        .rpc("channel.attempt", json!({"method": "channel.first"}))
        .await;
    assert_eq!(reply["result"], json!({"code": "gone"}), "{reply}");
    server.shutdown();
    server.join().await.unwrap();
}

/// A.2: an encoded notification over the frame limit is `TooLarge`; nothing
/// is written and the connection stays open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oversized_notification_is_refused_and_the_connection_stays_open() {
    let (server, mut client, _) = launch_channel().await;
    assert_ok(&client.rpc("channel.start", json!({})).await);
    let reply = client
        .rpc(
            "channel.attempt",
            json!({"method": "channel.first", "bytes": 16 * 1024 * 1024}),
        )
        .await;
    assert_eq!(reply["result"], json!({"code": "too_large"}), "{reply}");
    assert_ok(&client.rpc("daemon.ping", json!({})).await);
    assert!(
        client.notices.is_empty(),
        "an oversized notification was written: {:?}",
        client.notices.len()
    );
    server.shutdown();
    server.join().await.unwrap();
}

/// A.2: a method a Session namespace serves gets the same caller and
/// delivers the same way.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_namespace_method_delivers_connection_bound_notifications() {
    let (server, mut client, state) = launch_channel().await;
    let created = client.rpc("session.create", create(json!([]))).await;
    assert_ok(&created);
    let session = created["result"]["sessionId"].as_str().unwrap();
    let id = client
        .send("session.channel.stream", json!({"sessionId": session}))
        .await;
    let reply = client.read("the Session method reply").await;
    assert_eq!(reply["id"], id, "{reply}");
    assert_eq!(reply["result"], json!({"ok": true}));
    let first = client.read("the Session notification").await;
    assert_eq!(first["method"], "session.channel.first");
    state.release.notify_one();
    let second = client.read("the later Session notification").await;
    assert_eq!(second["method"], "session.channel.second");
    server.shutdown();
    server.join().await.unwrap();
}

/// A.2: the notifications one call holds are bounded by the connection's
/// reliable allowance. A call that holds past it is a slow consumer: the
/// connection closes, the reply is not written, and later notifications on
/// the same caller answer `gone`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_holding_past_the_allowance_closes_the_connection_as_a_slow_consumer() {
    let (server, mut client, state) = launch_channel().await;
    assert_ok(&client.rpc("channel.start", json!({})).await);
    client.send("channel.fill", json!({"count": 1100})).await;
    let Transport::Unix(stream) = &mut client.transport else {
        panic!("the channel fixture is served on the local entry");
    };
    let mut line = String::new();
    let read = tokio::time::timeout(Duration::from_secs(10), stream.read_line(&mut line))
        .await
        .expect("the slow consumer's connection closes");
    assert_eq!(
        read.unwrap(),
        0,
        "the connection stayed open and wrote: {line}"
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some((first, later)) = *state.filled.lock().unwrap() {
            assert_eq!(
                (first, later),
                ("gone", "gone"),
                "the first refusal and every later notification answer gone"
            );
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the call never reached the allowance's limit"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    server.shutdown();
    server.join().await.unwrap();
}

/// The error a provider sees is the contract's; kept here so the test checks
/// the enum rather than only its code.
#[test]
fn notify_error_codes_are_the_contracts() {
    assert_eq!(NotifyError::Undeclared.code(), "undeclared");
    assert_eq!(NotifyError::Gone.code(), "gone");
    assert_eq!(NotifyError::TooLarge.code(), "too_large");
}
