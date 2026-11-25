//! The non-force removal of an unregistered directory, over the local
//! transport the product uses. A platform without handle-relative deletion
//! refuses it, and `--force` remains the way to remove the tree.
use dim_agent_capabilities::defaults;
use dim_agent_runtime::{AgentBuilder, AgentConfig};
use dim_app_server::{ServerConfig, serve};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

fn builder() -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts(&mut builder);
    defaults::install(&mut builder);
    builder
}

async fn reply(
    client: &mut BufReader<NamedPipeClient>,
    id: u32,
    method: &str,
    params: Value,
) -> Value {
    let mut frame = serde_json::to_vec(&json!({
        "jsonrpc":"2.0", "id":id, "method":method, "params":params,
    }))
    .unwrap();
    frame.push(b'\n');
    client.get_mut().write_all(&frame).await.unwrap();
    loop {
        let mut line = String::new();
        assert_ne!(client.read_line(&mut line).await.unwrap(), 0);
        let reply: Value = serde_json::from_str(&line).unwrap();
        if reply["id"] == id {
            return reply;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_force_removal_refuses_an_unregistered_directory() {
    let home = std::env::temp_dir().join(format!("dim-worktree-removal-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let home = home.canonicalize().unwrap();
    let pipe = format!(r"\\.\pipe\dim-worktree-removal-{}", uuid::Uuid::new_v4());
    let server = serve(
        builder(),
        ServerConfig {
            pipe_name: Some(pipe.clone()),
            websocket_address: None,
            product_home: Some(home.clone()),
            target_home: Some(home.join("native")),
            platform_home: Some(home.join("user")),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let entry = home.join("worktrees").join("aaaaaa");
    std::fs::create_dir_all(entry.join("repo")).unwrap();
    let mut client = BufReader::new(ClientOptions::new().open(&pipe).unwrap());
    let hello = reply(
        &mut client,
        1,
        "hello",
        json!({"protocolVersion":3,"clientKind":"cli"}),
    )
    .await;
    assert_eq!(hello["result"]["protocolVersion"], 3);
    let refused = reply(
        &mut client,
        2,
        "worktree.remove",
        json!({"taskId":"aaaaaa"}),
    )
    .await;
    assert_eq!(refused["error"]["data"]["appError"], "worktree_in_use");
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--force"),
        "{refused}"
    );
    assert!(entry.join("repo").is_dir());
    let removed = reply(
        &mut client,
        3,
        "worktree.remove",
        json!({"taskId":"aaaaaa","force":true}),
    )
    .await;
    assert_eq!(removed["result"]["removed"], true);
    assert!(!entry.exists());
    server.shutdown();
    server.join().await.unwrap();
    let _ = std::fs::remove_dir_all(&home);
}
