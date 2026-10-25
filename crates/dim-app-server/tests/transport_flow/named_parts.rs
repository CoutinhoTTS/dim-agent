//! Name-bound Rust parts across languages (plugin contract P2, rutis usage
//! §3.5): a Rust Session part provides a crossing name that a TypeScript
//! row reads, and a Rust branch part reads a name that TypeScript row
//! provides. Each Session's parts see their own Session's copies.
use super::session_typescript::{fixture, new_session, setup};
use super::*;
use dim_agent_runtime::{SESSION_GROUP, SessionIdentity, TimelineIdentity};
use rutis::{BoxFuture, CordisError, Ctx, DepKey, Effect, Keyed, TypedPlugin};
use rutis_bridge::session::{HostDispatch, Reply, Value as RpcValue};
use std::sync::Mutex;

type Log = Arc<Mutex<Vec<String>>>;

/// `testGreeter` of one Session, served to other languages by name.
struct Greeter(String);
impl HostDispatch for Greeter {
    fn invoke(&self, method: &str, _: RpcValue) -> Reply {
        assert_eq!(method, "hello");
        Ok(json!(format!("hello {}", self.0)).into())
    }
    fn methods(&self) -> Option<Value> {
        Some(json!({"hello":"sync"}))
    }
}
struct GreeterPart(DepKey<dyn HostDispatch>);
impl TypedPlugin for GreeterPart {
    type Deps = (Keyed<SessionIdentity>,);
    fn name(&self) -> &str {
        "test.named-greeter"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(identity),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as::<dyn HostDispatch>(
                self.0.key().clone(),
                Arc::new(Greeter(identity.session_id.clone())),
            )?;
            Ok(Effect::Done)
        })
    }
}

/// Reads the TypeScript row's `sessionMethods:session.greeter` from each
/// branch, at the name's catalog key.
struct MethodsReader(Log);
impl TypedPlugin for MethodsReader {
    type Deps = (Keyed<dyn HostDispatch>, Keyed<TimelineIdentity>);
    fn name(&self) -> &str {
        "test.named-methods-reader"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        (Keyed(methods), Keyed(branch)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(format!(
                "{} {} for {}",
                methods.provider().unwrap_or_default(),
                methods.methods().unwrap_or_default(),
                branch.session_id
            ));
            Ok(Effect::Done)
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_named_parts_share_services_across_languages_per_session() {
    let log = Log::default();
    let (mut builder, _requests, _release) = setup();
    builder
        .catalog_mut()
        .register_shared_instance("testGreeter", SESSION_GROUP);
    let read = log.clone();
    builder
        .session_part(|names| names.part(GreeterPart(names.named("testGreeter")?)))
        .branch_part(move |names| {
            Ok(names.part_with(
                MethodsReader(read.clone()),
                (
                    names.named("sessionMethods:session.greeter")?,
                    names.key::<TimelineIdentity>()?,
                ),
            ))
        });
    let mut parts = fixture();
    let root = parts.anchor.parent().unwrap().join("fixtures");
    parts.rows = vec![json!({
        "id": "named-greeter",
        "name": root.join("named-greeter.ts"),
        "group": "session",
        "inject": ["testGreeter"],
        "provides": serde_json::from_str::<Value>(include_str!("../../../../runtime/fixtures/named-greeter.json")).unwrap(),
        "config": {},
    })];
    let server = serve(
        builder,
        ServerConfig {
            unix_path: Some(std::env::temp_dir().join(format!(
                "dm-np-{}/server.sock",
                &uuid::Uuid::new_v4().simple().to_string()[..8]
            ))),
            session_typescript: Some(parts),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut client = Client::new_version(&server, false, 3).await;
    let mut ids = Vec::new();
    for _ in 0..2 {
        ids.push(new_session(&mut client, json!(["echo"])).await);
    }
    for id in &ids {
        // Rust provides, TypeScript reads: the row asks its own Session's
        // Rust greeter.
        let asked = client
            .rpc("session.greeter.ask", json!({"sessionId":id}))
            .await;
        assert_ok(&asked);
        assert_eq!(asked["result"], json!(format!("hello {id}")));
    }
    // TypeScript provides, Rust reads: each Session's main branch reached
    // its own Session's row.
    let read = log.lock().unwrap().clone();
    for id in &ids {
        assert!(
            read.iter()
                .any(|entry| entry.starts_with("row named-greeter ")
                    && entry.contains("describe")
                    && entry.ends_with(&format!(" for {id}"))),
            "{read:?}"
        );
    }
    assert_eq!(read.len(), 2, "{read:?}");
    server.shutdown();
    server.join().await.unwrap();
}
