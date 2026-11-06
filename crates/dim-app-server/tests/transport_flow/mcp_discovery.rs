//! Discovery of other clients' known MCP stores, and the `from` import input.
//!
//! The daemon is spawned with a disposable `HOME`, because the stores are
//! resolved from the process environment; every fixture value is distinct so a
//! reply that carried one would be caught.

use super::*;
use serde_json::Map;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

const CLAUDE_ENV: &str = "claude-secret-environment";
const CLAUDE_ARGUMENT: &str = "claude-secret-argument";
const CODEX_ENV: &str = "codex-secret-environment";
const CODEX_HEADER: &str = "codex-secret-header";
const OMP_HEADER: &str = "omp-secret-header";
const OTHER_PROJECT: &str = "other-project-server";
const SECRETS: [&str; 5] = [
    CLAUDE_ENV,
    CLAUDE_ARGUMENT,
    CODEX_ENV,
    CODEX_HEADER,
    OMP_HEADER,
];

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    workspace: PathBuf,
    state: PathBuf,
}
impl Fixture {
    fn new(_name: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        // The daemon's unix socket must fit in `sun_path`, so the fixture root
        // stays short rather than nesting under the target directory.
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let root = std::env::temp_dir().join(format!("dim-disc-{}", &suffix[..8]));
        let home = root.join("home");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            state: root.join("state"),
            root,
            home,
            workspace,
        }
    }

    fn write(&self, relative: &str, text: &str) {
        write(&self.home.join(relative), text);
    }

    fn write_workspace(&self, relative: &str, text: &str) {
        write(&self.workspace.join(relative), text);
    }

    fn cwd(&self) -> String {
        self.workspace.to_str().unwrap().to_owned()
    }

    fn claude(&self) {
        let canonical = std::fs::canonicalize(&self.workspace).unwrap();
        let mut projects = Map::new();
        projects.insert(
            canonical.to_str().unwrap().to_owned(),
            json!({"mcpServers": {
                "claude-local": {"command": "echo", "args": [CLAUDE_ARGUMENT]}
            }}),
        );
        projects.insert(
            "/other/project".to_owned(),
            json!({"mcpServers": {OTHER_PROJECT: {"command": "echo"}}}),
        );
        self.write(
            ".claude.json",
            &json!({
                "account": {"email": "user@example.invalid"},
                "mcpServers": {
                    "claude-user": {
                        "command": "echo",
                        "args": ["--user"],
                        "env": {"TOKEN": CLAUDE_ENV}
                    }
                },
                "projects": projects
            })
            .to_string(),
        );
    }

    fn codex(&self) {
        self.write(
            ".codex/config.toml",
            &format!(
                r#"
[mcp_servers.codex-user]
command = "echo"
args = ["--codex"]
env = {{ TOKEN = "{CODEX_ENV}" }}
tool_timeout_sec = 90
startup_timeout_sec = 5

[mcp_servers.codex-remote]
url = "https://example.invalid/mcp"
bearer_token_env_var = "CODEX_TOKEN"
http_headers = {{ "X-Static" = "{CODEX_HEADER}" }}
"#
            ),
        );
    }

    fn omp(&self) {
        self.write(
            ".omp/agent/mcp.json",
            &json!({
                "$schema": "https://example.invalid/schema.json",
                "mcpServers": {
                    "omp-user": {
                        "type": "http",
                        "url": "https://example.invalid/omp",
                        "headers": {"Authorization": format!("Bearer {OMP_HEADER}")}
                    }
                },
                "disabledServers": ["omp-user"]
            })
            .to_string(),
        );
    }

    fn project_stores(&self) {
        self.write_workspace(
            ".codex/config.toml",
            "[mcp_servers.codex-project]\ncommand = \"echo\"\n",
        );
        self.write_workspace(
            ".omp/mcp.json",
            &json!({"mcpServers": {"omp-project": {"command": "echo"}}}).to_string(),
        );
    }

    /// A development model profile: protected, so the daemon serves its
    /// management namespace; no turn ever reaches this provider.
    fn model_config(&self) -> PathBuf {
        let path = self.root.join("model.json");
        write(
            &path,
            &json!({
                "defaultModel": {"providerId": "local", "modelId": "main"},
                "providers": [{
                    "id": "local",
                    "kind": "openai-compatible",
                    "apiKeyEnv": null,
                    "baseUrl": "http://127.0.0.1:1/v1",
                    "models": [{
                        "id": "main",
                        "contextWindow": 128000,
                        "maxOutputTokens": 16384,
                        "config": {"maxTokens": 16384}
                    }]
                }]
            })
            .to_string(),
        );
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

struct Server {
    child: tokio::process::Child,
    socket: PathBuf,
}
async fn launch(fixture: &Fixture) -> Server {
    let socket = fixture.root.join("server.sock");
    let config = fixture.model_config();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_dim-app-server"))
        .args([
            "--model-config",
            config.to_str().unwrap(),
            "--unix",
            socket.to_str().unwrap(),
            "--no-ws",
            "--home",
            fixture.state.to_str().unwrap(),
        ])
        .env("HOME", &fixture.home)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .current_dir(&fixture.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).await.unwrap();
    assert!(!line.is_empty(), "server startup failed");
    Server { child, socket }
}
impl Server {
    async fn client(&self) -> Client {
        let mut client = Client {
            transport: Transport::Unix(BufReader::new(
                UnixStream::connect(&self.socket).await.unwrap(),
            )),
            id: 0,
            notices: vec![],
            instance: Value::Null,
            features: Value::Null,
        };
        let hello = client
            .rpc("hello", json!({"protocolVersion":3,"clientKind":"other"}))
            .await;
        assert_ok(&hello);
        client.instance = hello["result"]["instanceId"].clone();
        client.features = hello["result"]["features"].clone();
        client
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

async fn read_sources(client: &mut Client, cwd: Option<&str>) -> Value {
    let params = match cwd {
        Some(cwd) => json!({"cwd": cwd}),
        None => json!({}),
    };
    let reply = client.rpc("mcp.import.sources", params).await;
    assert_ok(&reply);
    reply["result"]["sources"].clone()
}

fn row<'a>(sources: &'a Value, id: &str) -> &'a Value {
    sources
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("no source {id}: {sources}"))
}

fn ids(sources: &Value) -> Vec<&str> {
    sources
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect()
}

fn assert_no_value(reply: &Value, secrets: &[&str]) {
    let text = reply.to_string();
    for secret in secrets {
        assert!(!text.contains(secret), "{secret} leaked into {text}");
    }
}

fn entry_ids(preview: &Value) -> Vec<&str> {
    preview["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect()
}

/// A preview's result without the store digest, so a `from` preview and the
/// same converted text sent as `document` compare equal.
/// The preview's result without the members that name its input: a `from`
/// preview carries the store's digest and row, which a `document` preview of
/// the same text does not.
fn preview_result(mut preview: Value) -> Value {
    if let Some(result) = preview["result"].as_object_mut() {
        result.remove("sourceRevision");
        result.remove("store");
    }
    preview["result"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sources_reports_every_known_store_without_a_value() {
    let fixture = Fixture::new("dm-discovery-sources");
    fixture.claude();
    fixture.codex();
    fixture.omp();
    fixture.project_stores();
    let server = launch(&fixture).await;
    let mut client = server.client().await;
    let cwd = fixture.cwd();

    for method in ["mcp.import.sources", "mcp.import.preview"] {
        assert!(client.features.as_array().unwrap().contains(&json!(method)));
    }
    let reply = client.rpc("mcp.import.sources", json!({"cwd": cwd})).await;
    assert_ok(&reply);
    let sources = reply["result"]["sources"].clone();
    assert_eq!(
        ids(&sources),
        [
            "claude-code:user",
            "claude-code:local",
            "codex:user",
            "codex:project",
            "omp:user",
            "omp:project"
        ]
    );
    for source in sources.as_array().unwrap() {
        assert_eq!(source["state"], json!("ready"), "{source}");
        assert!(source["entries"].is_u64(), "{source}");
        assert!(source.get("error").is_none(), "{source}");
    }
    assert_eq!(row(&sources, "claude-code:user")["entries"], json!(1));
    assert_eq!(row(&sources, "claude-code:local")["entries"], json!(1));
    assert_eq!(row(&sources, "codex:user")["entries"], json!(2));
    assert_eq!(row(&sources, "codex:project")["entries"], json!(1));
    assert_eq!(row(&sources, "omp:user")["entries"], json!(1));
    assert_eq!(row(&sources, "omp:project")["entries"], json!(1));
    assert_eq!(
        row(&sources, "claude-code:user")["path"],
        json!("~/.claude.json")
    );
    assert_eq!(
        row(&sources, "claude-code:user")["client"],
        json!("claude-code")
    );
    assert_eq!(row(&sources, "claude-code:local")["scope"], json!("local"));
    let local = row(&sources, "claude-code:local")["path"].as_str().unwrap();
    assert!(local.starts_with("~/.claude.json · projects["), "{local}");
    assert_eq!(
        row(&sources, "codex:user")["path"],
        json!("~/.codex/config.toml")
    );
    assert_eq!(
        row(&sources, "codex:project")["path"],
        json!(".codex/config.toml")
    );
    assert_eq!(
        row(&sources, "omp:user")["path"],
        json!("~/.omp/agent/mcp.json")
    );
    assert_eq!(row(&sources, "omp:project")["path"], json!(".omp/mcp.json"));
    assert_no_value(&reply, &SECRETS);
    assert!(!reply.to_string().contains(OTHER_PROJECT));

    let user_only = client.rpc("mcp.import.sources", json!({})).await;
    assert_ok(&user_only);
    assert_eq!(
        ids(&user_only["result"]["sources"]),
        ["claude-code:user", "codex:user", "omp:user"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_store_keeps_its_own_state_and_an_empty_store_is_ready() {
    let fixture = Fixture::new("dm-discovery-states");
    fixture.write(".claude.json", &json!({"mcpServers": {}}).to_string());
    fixture.write(".codex/config.toml", "not toml [");
    fixture.write(".omp/agent/mcp.json", "{\"mcpServers\":");
    let server = launch(&fixture).await;
    let mut client = server.client().await;
    let cwd = fixture.cwd();
    let sources = read_sources(&mut client, Some(&cwd)).await;

    let user = row(&sources, "claude-code:user");
    assert_eq!(user["state"], json!("ready"), "{user}");
    assert_eq!(user["entries"], json!(0), "{user}");
    let local = row(&sources, "claude-code:local");
    assert_eq!(local["state"], json!("absent"), "{local}");
    assert!(local.get("error").is_none(), "{local}");
    let codex = row(&sources, "codex:user");
    assert_eq!(codex["state"], json!("malformed"), "{codex}");
    assert!(codex["error"].as_str().unwrap().contains("line"), "{codex}");
    let omp = row(&sources, "omp:user");
    assert_eq!(omp["state"], json!("malformed"), "{omp}");
    assert!(omp["error"].as_str().unwrap().contains("line"), "{omp}");
    assert_eq!(row(&sources, "codex:project")["state"], json!("absent"));
    assert_eq!(row(&sources, "omp:project")["state"], json!("absent"));

    let missing = Fixture::new("dm-discovery-missing");
    let server = launch(&missing).await;
    let mut client = server.client().await;
    let sources = read_sources(&mut client, Some(&missing.cwd())).await;
    assert_eq!(row(&sources, "claude-code:user")["state"], json!("absent"));
    assert!(row(&sources, "claude-code:user").get("error").is_none());

    let large = Fixture::new("dm-discovery-large");
    let file_path = large.home.join(".omp/agent/mcp.json");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::File::create(&file_path)
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    let server = launch(&large).await;
    let mut client = server.client().await;
    let sources = read_sources(&mut client, None).await;
    let row = row(&sources, "omp:user");
    assert_eq!(row["state"], json!("too-large"), "{row}");
    assert!(row.get("error").is_none(), "{row}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_and_apply_read_one_store_in_the_daemon() {
    let fixture = Fixture::new("dm-discovery-import");
    fixture.claude();
    fixture.codex();
    fixture.omp();
    fixture.project_stores();
    let server = launch(&fixture).await;
    let mut client = server.client().await;
    let cwd = fixture.cwd();

    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "user", "from": "codex:user"}),
        )
        .await;
    assert_ok(&preview);
    assert_no_value(&preview, &SECRETS);
    assert_eq!(entry_ids(&preview), ["codex-user", "codex-remote"]);
    let source_revision = preview["result"]["sourceRevision"]
        .as_str()
        .unwrap_or_else(|| panic!("no source revision: {preview}"))
        .to_owned();

    // The Codex fixture's converted document, written out: previewing it as
    // `document` answers exactly what the `from` preview answered, so the
    // wire carries the mapping and not just the same ids.
    let converted = json!({
        "mcpServers": {
            "codex-user": {
                "command": "echo",
                "args": ["--codex"],
                "env": {"TOKEN": CODEX_ENV},
                "timeout": 90_000,
                "startup_timeout_sec": 5
            },
            "codex-remote": {
                "url": "https://example.invalid/mcp",
                "type": "http",
                "auth": {"type": "bearer", "token": "${CODEX_TOKEN}"},
                "headers": {"X-Static": CODEX_HEADER}
            }
        }
    })
    .to_string();
    let document = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "user", "document": converted}),
        )
        .await;
    assert_ok(&document);
    assert_no_value(&document, &SECRETS);
    assert_eq!(preview_result(document), preview_result(preview.clone()));

    // OMP converts its document as it stands, so its other top-level members
    // are reported as unknown and the entries match the same text.
    let omp = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "user", "from": "omp:user"}),
        )
        .await;
    assert_ok(&omp);
    assert_no_value(&omp, &SECRETS);
    let converted = json!({
        "$schema": "https://example.invalid/schema.json",
        "mcpServers": {
            "omp-user": {
                "type": "http",
                "url": "https://example.invalid/omp",
                "headers": {"Authorization": format!("Bearer {OMP_HEADER}")}
            }
        },
        "disabledServers": ["omp-user"]
    })
    .to_string();
    let document = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "user", "document": converted}),
        )
        .await;
    assert_ok(&document);
    assert_no_value(&document, &SECRETS);
    let converted_result = preview_result(document);
    assert_eq!(
        converted_result["unknownTopLevel"],
        json!(["$schema", "disabledServers"])
    );
    assert_eq!(converted_result, preview_result(omp));

    let applied = client
        .rpc(
            "mcp.import.apply",
            json!({
                "cwd": cwd,
                "source": "user",
                "from": "codex:user",
                "expectedVersion": preview["result"]["revision"],
                "expectedSourceRevision": source_revision,
                "resolutions": [
                    {"id": "codex-user", "action": "import"},
                    {"id": "codex-remote", "action": "import"}
                ]
            }),
        )
        .await;
    assert_ok(&applied);
    assert_no_value(&applied, &SECRETS);

    let catalog = client.rpc("mcp.catalog", json!({})).await;
    assert_ok(&catalog);
    let servers = catalog["result"]["servers"].as_array().unwrap();
    for id in ["codex-user", "codex-remote"] {
        let server = servers
            .iter()
            .find(|server| server["id"] == id)
            .unwrap_or_else(|| panic!("no catalog entry {id}: {catalog}"));
        assert_eq!(server["enabled"], json!(false), "{server}");
    }
    assert_no_value(&catalog, &SECRETS);

    let configuration = client
        .rpc(
            "mcp.configuration",
            json!({"source": "user", "id": "codex-user"}),
        )
        .await;
    assert_ok(&configuration);
    assert_eq!(
        configuration["result"]["configuration"]["timeout"],
        json!(90_000)
    );
    assert!(
        configuration["result"]["configuration"]["envNames"]
            .as_array()
            .unwrap()
            .contains(&json!("TOKEN"))
    );
    assert_no_value(&configuration, &SECRETS);

    let remote = client
        .rpc(
            "mcp.configuration",
            json!({"source": "user", "id": "codex-remote"}),
        )
        .await;
    assert_ok(&remote);
    assert_eq!(
        remote["result"]["configuration"]["authConfigured"],
        json!(true)
    );
    assert_eq!(
        remote["result"]["configuration"]["endpoint"],
        json!("https://example.invalid")
    );
    assert!(
        remote["result"]["configuration"]["referencedVariables"]
            .as_array()
            .unwrap()
            .contains(&json!("CODEX_TOKEN"))
    );
    assert_no_value(&remote, &SECRETS);

    // A project-scoped store is read with the same authorized workspace and
    // imported into the project source.
    let project = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "project", "from": "codex:project"}),
        )
        .await;
    assert_ok(&project);
    assert_eq!(entry_ids(&project), ["codex-project"]);
    assert_no_value(&project, &SECRETS);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_store_that_changed_since_the_preview_refuses_the_apply() {
    let fixture = Fixture::new("dm-discovery-changed");
    fixture.codex();
    let server = launch(&fixture).await;
    let mut client = server.client().await;
    let cwd = fixture.cwd();
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "user", "from": "codex:user"}),
        )
        .await;
    assert_ok(&preview);
    let source_revision = preview["result"]["sourceRevision"].as_str().unwrap();
    fixture.write(
        ".codex/config.toml",
        "[mcp_servers.codex-user]\ncommand = \"echo\"\n\n[mcp_servers.later]\ncommand = \"echo\"\n",
    );
    let applied = client
        .rpc(
            "mcp.import.apply",
            json!({
                "cwd": cwd,
                "source": "user",
                "from": "codex:user",
                "expectedVersion": preview["result"]["revision"],
                "expectedSourceRevision": source_revision,
                "resolutions": [{"id": "codex-user", "action": "import"}]
            }),
        )
        .await;
    assert_eq!(
        applied["error"]["data"]["appError"],
        json!("mcp_settings_conflict"),
        "{applied}"
    );
    assert!(
        applied["error"]["message"]
            .as_str()
            .unwrap()
            .contains("store changed"),
        "{applied}"
    );
    let catalog = client.rpc("mcp.catalog", json!({})).await;
    assert_ok(&catalog);
    assert!(
        catalog["result"]["servers"].as_array().unwrap().is_empty(),
        "{catalog}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_project_store_outside_the_workspace_is_never_read() {
    let fixture = Fixture::new("dm-discovery-outside");
    fixture.claude();
    std::fs::create_dir_all(fixture.workspace.join(".codex")).unwrap();
    std::os::unix::fs::symlink(
        fixture.home.join(".claude.json"),
        fixture.workspace.join(".codex/config.toml"),
    )
    .unwrap();
    let server = launch(&fixture).await;
    let mut client = server.client().await;
    let cwd = fixture.cwd();
    let reply = client.rpc("mcp.import.sources", json!({"cwd": cwd})).await;
    assert_ok(&reply);
    let sources = reply["result"]["sources"].clone();
    let row = row(&sources, "codex:project");
    assert_eq!(row["state"], json!("unreadable"), "{row}");
    assert!(
        row["error"]
            .as_str()
            .unwrap()
            .contains("outside the workspace"),
        "{row}"
    );
    assert_no_value(&reply, &SECRETS);

    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "project", "from": "codex:project"}),
        )
        .await;
    assert_eq!(
        preview["error"]["data"]["appError"],
        json!("mcp_settings_read"),
        "{preview}"
    );
    assert_no_value(&preview, &SECRETS);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn import_inputs_are_exactly_one_of_document_and_from() {
    let fixture = Fixture::new("dm-discovery-inputs");
    fixture.codex();
    let server = launch(&fixture).await;
    let mut client = server.client().await;
    let cwd = fixture.cwd();
    let document = json!({"mcpServers": {"x": {"command": "echo"}}}).to_string();
    let cases = [
        json!({"source": "user", "document": document.clone(), "from": "codex:user"}),
        json!({"source": "user"}),
        json!({"source": "user", "from": "unknown:store"}),
        json!({"source": "user", "from": "codex:user", "expectedSourceRevision": "x"}),
    ];
    for params in cases {
        let mut params = params.as_object().unwrap().clone();
        params.insert("cwd".to_owned(), json!(cwd));
        let reply = client
            .rpc("mcp.import.preview", Value::Object(params))
            .await;
        assert_eq!(reply["error"]["code"], json!(-32602), "{reply}");
    }
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "user", "from": "codex:user"}),
        )
        .await;
    assert_ok(&preview);
    let expected = preview["result"]["revision"].clone();
    let pairs = [
        (
            json!({"source": "user", "from": "codex:user"}),
            "an import from a source takes expectedSourceRevision",
        ),
        (
            json!({"source": "user", "document": document.clone(), "expectedSourceRevision": "x"}),
            "expectedSourceRevision is only for an import from a source",
        ),
        (
            json!({"source": "user", "from": "codex:project", "expectedSourceRevision": "x"}),
            "Project MCP sources require cwd",
        ),
    ];
    for (params, message) in pairs {
        let mut params = params.as_object().unwrap().clone();
        params.insert("expectedVersion".to_owned(), expected.clone());
        params.insert("resolutions".to_owned(), json!([]));
        let reply = client.rpc("mcp.import.apply", Value::Object(params)).await;
        assert_eq!(reply["error"]["code"], json!(-32602), "{reply}");
        assert_eq!(reply["error"]["message"], json!(message), "{reply}");
    }
}

/// A preview that read a store names it, with the row `mcp.import.sources`
/// reports; a preview of the client's own document names none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_preview_names_the_store_it_read() {
    let fixture = Fixture::new("dm-discovery-preview-source");
    fixture.claude();
    fixture.codex();
    fixture.omp();
    let server = launch(&fixture).await;
    let mut client = server.client().await;
    let cwd = fixture.cwd();

    let sources = read_sources(&mut client, Some(&cwd)).await;
    let codex = row(&sources, "codex:user").clone();
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "user", "from": "codex:user"}),
        )
        .await;
    assert_ok(&preview);
    let store = &preview["result"]["store"];
    assert_eq!(store["id"], codex["id"], "{preview}");
    assert_eq!(store["client"], codex["client"], "{preview}");
    assert_eq!(store["scope"], codex["scope"], "{preview}");
    assert_eq!(store["path"], codex["path"], "{preview}");
    assert!(store.get("entries").is_none(), "{store}");
    assert!(store.get("state").is_none(), "{store}");
    assert_no_value(&preview, &SECRETS);

    // The local Claude row includes the `projects` record it matched, and the
    // preview carries that same row.
    let local = row(&sources, "claude-code:local").clone();
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "user", "from": "claude-code:local"}),
        )
        .await;
    assert_ok(&preview);
    let store = &preview["result"]["store"];
    assert_eq!(store["id"], local["id"], "{preview}");
    assert_eq!(store["client"], local["client"], "{preview}");
    assert_eq!(store["scope"], local["scope"], "{preview}");
    assert_eq!(store["path"], local["path"], "{preview}");
    assert!(
        store["path"].as_str().unwrap().contains("· projects["),
        "{store}"
    );
    assert_no_value(&preview, &SECRETS);

    let document = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "user", "document": "{\"mcpServers\":{}}"}),
        )
        .await;
    assert_ok(&document);
    assert!(
        document["result"].get("store").is_none(),
        "a document preview names no store: {document}"
    );
}

/// A preview reads only the store it names: a different store unreadable in
/// the environment neither fails the preview nor changes the row it reports.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_preview_reads_only_the_store_it_names() {
    let fixture = Fixture::new("dm-discovery-preview-one");
    fixture.codex();
    fixture.omp();
    std::fs::create_dir(fixture.home.join(".claude.json")).unwrap();
    let server = launch(&fixture).await;
    let mut client = server.client().await;
    let cwd = fixture.cwd();

    let sources = read_sources(&mut client, Some(&cwd)).await;
    assert_eq!(
        row(&sources, "claude-code:user")["state"],
        json!("unreadable"),
        "{sources}"
    );
    let codex = row(&sources, "codex:user").clone();
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"cwd": cwd, "source": "user", "from": "codex:user"}),
        )
        .await;
    assert_ok(&preview);
    let store = &preview["result"]["store"];
    assert_eq!(store["id"], codex["id"], "{preview}");
    assert_eq!(store["client"], codex["client"], "{preview}");
    assert_eq!(store["scope"], codex["scope"], "{preview}");
    assert_eq!(store["path"], codex["path"], "{preview}");
    assert_no_value(&preview, &SECRETS);
}
