//! The `models` tool through an actual Session, ToolRegistry and ToolExecutor:
//! it lists the models Provider would serve, with their provider IDs.
use super::*;

const PROVIDER: &str = "018ec73f-0000-7000-8000-000000000051";

async fn create_session(client: &mut Client) -> String {
    let created = client
        .rpc(
            "session.create",
            json!({"cwd": std::env::temp_dir(), "tools": ["models"], "interactive": false}),
        )
        .await;
    assert_ok(&created);
    created["result"]["sessionId"].as_str().unwrap().to_owned()
}

async fn submit(client: &mut Client, id: &str) {
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":id,"branchId":"main","text":"tool:models {}"}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc("session.idle", json!({"sessionId":id,"branchId":"main"}))
            .await,
    );
}

async fn result(session: &dim_agent_runtime::Session) -> ToolResult {
    session
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .rev()
        .find_map(|event| {
            if let EventKind::ToolResult(batch) = &event.kind {
                batch.results.first().map(|entry| entry.result.clone())
            } else {
                None
            }
        })
        .expect("tool result")
}

fn text(result: &ToolResult) -> &str {
    match &result.content {
        dim_agent_types::ToolContent::Text(text) => text,
        _ => panic!("models returns text"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn models_tool_lists_the_provider_ids_the_session_can_use() {
    let home = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("dm-models-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let (server, _requests, _release) = super::session_typescript::start(ServerConfig {
        product_home: Some(home.clone()),
        target_home: Some(home.join("rust")),
        ..Default::default()
    })
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    assert_ok(
        &client
            .rpc(
                "provider.create",
                json!({"providerId":PROVIDER,"displayName":"Models fixture",
            "connection":{"driverKind":"openai-compatible","baseUrl":"http://localhost/v1","auth":{"type":"anonymous"}},
            "models":[
                {"modelId":"served","displayName":"Served","status":"active",
                 "capabilities":dim_agent_types::CatalogCapabilities::with_limits(128_000,16_384)},
                {"modelId":"gone","displayName":"Gone","status":"unavailable",
                 "capabilities":dim_agent_types::CatalogCapabilities::with_limits(64_000,8_192)}],
            "enabled":true}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc(
                "provider.select",
                json!({"providerId":PROVIDER,"modelId":"served"}),
            )
            .await,
    );
    let id = create_session(&mut client).await;
    let session = server.agent().unwrap().get_session(&id).unwrap();

    submit(&mut client, &id).await;
    let listed = result(&session).await;
    assert_ne!(listed.is_error, Some(true), "{listed:?}");
    assert_eq!(
        text(&listed),
        format!(
            "Available models: 1\n\nprovider {PROVIDER}\n  served  context 128000  output 16384"
        )
    );

    let area = client.rpc("provider.management", json!({})).await;
    assert_ok(&area);
    let version = area["result"]["providers"][0]["version"].clone();
    assert_ok(
        &client
            .rpc(
                "provider.models.set_enabled",
                json!({"providerId":PROVIDER,"expectedVersion":version,"modelIds":[]}),
            )
            .await,
    );
    submit(&mut client, &id).await;
    let none = result(&session).await;
    assert_ne!(none.is_error, Some(true), "{none:?}");
    assert_eq!(
        text(&none),
        "No models are available to this Session; sign in to a provider or add one first."
    );

    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}
