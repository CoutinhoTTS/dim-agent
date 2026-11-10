//! The context a capability appends to an operator input does not title the
//! Session: Hooks A.6.2 and Skills §9 add their blocks after the operator's
//! own, and the title is generated from the operator's words alone (#1378).
#![cfg(unix)]
mod support;

use axum::{Router, routing::post};
use dim_agent_capabilities::{adapters::MockTools, defaults};
use dim_agent_runtime::{AgentBuilder, AgentConfig};
use dim_app_server::{RunningServer, ServerConfig, serve};
use dim_client::Client;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use support::*;
use tokio::sync::mpsc;

const PROVIDER: &str = "018ec73f-0000-7000-8000-00000000000b";
const HOOK: &str = "<dim-hooks kind=\"prompt-submit\">The parser's entry points are in src/parser.rs; read it before answering, and keep the answer short.</dim-hooks>";
const SKILL: &str = "<dim-skill name=\"check\">\nSkill: check\nDescription: D\nFile: /tmp/check/SKILL.md\n\nInstructions:\nBODY\n</dim-skill>";
/// What the real `UserPromptSubmit` hook used by the empty-content flow
/// answers as its additional context.
const HOOK_CONTEXT: &str = "Read src/parser.rs before answering.";

fn builder(home: &std::path::Path) -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_session_title::install(&mut builder);
    defaults::install(&mut builder);
    defaults::mock_model_hosts(&mut builder);
    defaults::tool_host(&mut builder, Arc::new(MockTools));
    dim_app_server::collaboration::install(&mut builder);
    dim_app_server::hooks_settings::install(&mut builder, Some(home.to_owned()));
    dim_agent_capabilities::hooks::install(&mut builder);
    builder
}

/// A served product profile whose title requests answer through the returned
/// channel, as the local fixture stands in for the provider.
async fn start() -> (
    Box<Store>,
    RunningServer,
    Client,
    mpsc::UnboundedReceiver<Request>,
) {
    start_with(false).await
}

/// The `UserPromptSubmit` command the user's hooks run, if the Session gets
/// one: a script answering with `HOOK_CONTEXT` as its additional context. A
/// script keeps the command's own shell quoting out of the fixture.
fn hook(home: &std::path::Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    let script = home.join("prompt-context.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s' '{{\"hookSpecificOutput\":{{\"additionalContext\":\"{HOOK_CONTEXT}\"}}}}'\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script.to_string_lossy().into_owned()
}

/// [`start`] with the user's hooks document running the `UserPromptSubmit`
/// script `use_hook` writes, so the served Session runs a real hook.
async fn start_with(
    use_hook: bool,
) -> (
    Box<Store>,
    RunningServer,
    Client,
    mpsc::UnboundedReceiver<Request>,
) {
    let store = Box::new(Store::new());
    if use_hook {
        let command = hook(&store.0);
        std::fs::write(
            store.0.join("hooks.json"),
            json!({"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":command}]}]}})
                .to_string(),
        )
        .unwrap();
    }
    let (sent, requests) = mpsc::unbounded_channel::<Request>();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/{*path}", post(complete))
                .with_state(sent),
        )
        .await
        .unwrap();
    });
    let server = serve(
        builder(&store.0),
        ServerConfig {
            unix_path: Some(PathBuf::from("/tmp").join(format!(
                "dim-title-appended-{}/server.sock",
                uuid::Uuid::new_v4()
            ))),
            product_home: Some(store.0.clone()),
            ..store.config()
        },
    )
    .await
    .unwrap();
    let mut client = client(&server).await;
    client.rpc("provider.create", json!({"providerId":PROVIDER,"displayName":"Title fixture",
        "connection":{"driverKind":"openai-compatible","baseUrl":format!("http://{address}/v1"),"auth":{"type":"anonymous"}},
        "models":[{"modelId":"main","displayName":"Main","status":"active",
            "capabilities":dim_agent_types::CatalogCapabilities::with_limits(128_000,16_384)}],"enabled":true})).await.unwrap();
    client
        .rpc(
            "provider.select",
            json!({"providerId":PROVIDER,"modelId":"main"}),
        )
        .await
        .unwrap();
    configure(
        &mut client,
        json!({"enabled":true,"model":{"providerId":PROVIDER,"modelId":"main"}}),
    )
    .await;
    (store, server, client, requests)
}

async fn summary(client: &mut Client, id: &str) -> Value {
    client
        .rpc("session.summaries", json!({}))
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["sessionId"] == id)
        .expect("the Session is listed")
        .clone()
}

/// An interactive Session whose SessionTitle part has activated: `apply`
/// registers `session.rename` after it registers the observation.
async fn session(client: &mut Client, home: &std::path::Path) -> String {
    let created = client
        .rpc(
            "session.create",
            json!({"cwd":home,"tools":["wait"],"interactive":true,
            "permissionPolicy":{"preset":"full-access","allowedTools":[],"disallowedTools":[]}}),
        )
        .await
        .unwrap();
    let id = created["sessionId"].as_str().unwrap().to_owned();
    client
        .rpc("session.attach", json!({"sessionId":id}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let reply = client
                .rpc("session.rename", json!({"sessionId":id,"title":0}))
                .await
                .unwrap_err();
            if dim_client::app_error(&reply) == Some("invalid_params") {
                break;
            }
            assert!(waiting(&reply), "{reply}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the SessionTitle part activates");
    id
}

async fn submit(client: &mut Client, id: &str, content: Vec<&str>) {
    let content: Vec<Value> = content
        .into_iter()
        .map(|text| json!({"type":"text","text":text}))
        .collect();
    client
        .rpc(
            "session.submit",
            json!({"sessionId":id,"branchId":"main","user":{"content":content}}),
        )
        .await
        .unwrap();
}

/// Submits user content as given, which may be empty: an empty content list is
/// the input shape whose first block has no operator text before it (#1386).
async fn submit_content(
    client: &mut Client,
    id: &str,
    content: Vec<Value>,
) -> Result<Value, dim_client::Error> {
    client
        .rpc(
            "session.submit",
            json!({"sessionId":id,"branchId":"main","user":{"content":content}}),
        )
        .await
}

/// The text the title request carried and its release, once the request that
/// follows `submitted` reaches the provider.
async fn title_request(
    requests: &mut mpsc::UnboundedReceiver<Request>,
) -> (String, tokio::sync::oneshot::Sender<String>) {
    let (request, release) = tokio::time::timeout(Duration::from_secs(10), requests.recv())
        .await
        .expect("the adopted model receives the title request")
        .unwrap();
    let messages = request["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2, "{request}");
    (messages[1]["content"].as_str().unwrap().to_owned(), release)
}

async fn titled(client: &mut Client, id: &str, title: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while summary(client, id).await["title"] != title {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the committed title arrives");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hook_context_after_the_operators_text_does_not_title() {
    let (store, server, mut client, mut requests) = start().await;
    let id = session(&mut client, &store.0).await;
    submit(&mut client, &id, vec!["Why does the parser stall?", HOOK]).await;
    let (text, release) = title_request(&mut requests).await;
    assert_eq!(text, "Why does the parser stall?");
    release.send("Parser stall".into()).unwrap();
    titled(&mut client, &id, "Parser stall").await;
    assert!(requests.try_recv().is_err(), "one title attempt per input");
    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_operators_own_marker_in_the_first_block_still_titles() {
    let (store, server, mut client, mut requests) = start().await;
    let id = session(&mut client, &store.0).await;
    submit(&mut client, &id, vec![HOOK]).await;
    let (text, release) = title_request(&mut requests).await;
    assert_eq!(text, HOOK);
    release.send("Hook text".into()).unwrap();
    titled(&mut client, &id, "Hook text").await;
    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_loaded_skill_after_the_operators_text_does_not_title() {
    let (store, server, mut client, mut requests) = start().await;
    let id = session(&mut client, &store.0).await;
    submit(&mut client, &id, vec!["$skill: check the parser", SKILL]).await;
    let (text, release) = title_request(&mut requests).await;
    assert_eq!(text, "$skill: check the parser");
    release.send("Parser check".into()).unwrap();
    titled(&mut client, &id, "Parser check").await;
    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_input_without_appended_context_titles_unchanged() {
    let (store, server, mut client, mut requests) = start().await;
    let id = session(&mut client, &store.0).await;
    submit(&mut client, &id, vec!["Just the operator's own words"]).await;
    let (text, release) = title_request(&mut requests).await;
    assert_eq!(text, "Just the operator's own words");
    release.send("Own words".into()).unwrap();
    titled(&mut client, &id, "Own words").await;
    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}

/// #1386: an input with no operator content has nothing to append a context
/// after, so the hook's context is dropped rather than placed at index 0,
/// where every reader takes it for the operator's words. Such an input carries
/// no user content at all, so the entry refuses it exactly as it does with no
/// hook installed: nothing is written, nothing is titled, no preview is
/// decided.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_input_with_no_operator_content_takes_no_context_and_writes_nothing() {
    let (store, server, mut client, mut requests) = start_with(true).await;
    let id = session(&mut client, &store.0).await;
    let refused = submit_content(&mut client, &id, vec![])
        .await
        .expect_err("an input with no operator content is refused");
    assert_eq!(
        dim_client::app_error(&refused),
        Some("session_invalid_input"),
        "{refused}"
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(5), requests.recv())
            .await
            .is_err(),
        "a context the operator did not write titles nothing"
    );
    let preview = summary(&mut client, &id).await["preview"].clone();
    assert!(preview.is_null(), "no preview: {preview}");
    let history = client
        .rpc(
            "session.history",
            json!({"sessionId":id,"branchId":"main","after":0,"limit":64}),
        )
        .await
        .unwrap();
    assert!(
        !history["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["name"] == "message" && event["data"].get("user").is_some()),
        "the refused input wrote nothing: {history}"
    );
    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}
