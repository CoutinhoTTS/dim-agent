use super::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn servers_document(servers: Value) -> String {
    json!({"mcpServers": servers}).to_string()
}
fn entry<'a>(preview: &'a Value, id: &str) -> &'a Value {
    preview["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == id)
        .unwrap_or_else(|| panic!("no preview entry {id}: {preview}"))
}

fn fields(value: &Value) -> Vec<&str> {
    value["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| change["field"].as_str().unwrap())
        .collect()
}

fn unsupported(value: &Value) -> Vec<(&str, &str)> {
    value["unsupported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| {
            (
                field["field"].as_str().unwrap(),
                field["effect"].as_str().unwrap(),
            )
        })
        .collect()
}

fn catalog_entry<'a>(catalog: &'a Value, id: &str) -> &'a Value {
    catalog["result"]["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|server| server["id"] == id)
        .unwrap_or_else(|| panic!("no catalog entry {id}: {catalog}"))
}

async fn user_revision(client: &mut Client) -> Value {
    let catalog = client.rpc("mcp.catalog", json!({})).await;
    assert_ok(&catalog);
    catalog["result"]["userRevision"].clone()
}

async fn save(client: &mut Client, id: &str, configuration: Value) {
    let revision = user_revision(client).await;
    let saved = client
        .rpc(
            "mcp.save",
            json!({"source":"user","id":id,"expectedVersion":revision,"configuration":configuration}),
        )
        .await;
    assert_ok(&saved);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_previews_selects_and_writes_multiple_user_entries() {
    let home = scratch("dm-import-user");
    let (server, _) = launch_with_agent(
        Arc::new(MockTools),
        ServerConfig {
            product_home: Some(home.clone()),
            target_home: Some(home.join("native")),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let mut remote = Client::new_version(&server, true, 3).await;
    for method in ["mcp.import.preview", "mcp.import.apply"] {
        assert!(client.features.as_array().unwrap().contains(&json!(method)));
        assert_eq!(
            remote.rpc(method, json!({})).await["error"]["data"]["appError"],
            "local_transport_required"
        );
    }
    save(
        &mut client,
        "github",
        json!({"command":"echo","args":["--old"]}),
    )
    .await;
    save(
        &mut client,
        "skipme",
        json!({"command":"echo","args":["--old","--keep"],"enabled":false}),
    )
    .await;
    save(
        &mut client,
        "dup",
        json!({"command":"echo","args":["--old","--keep"],"enabled":false}),
    )
    .await;
    save(
        &mut client,
        "same",
        json!({"command":"echo","enabled":false}),
    )
    .await;
    save(&mut client, "keeps", json!({"command":"echo"})).await;
    let document = json!({"mcpServers": {
        "fresh": {"command":"echo","args":["--title","oauth-secret-argument"],"env":{"MODE":"oauth-secret-environment"},"enabled":true,"extensionField":true},
        "github": {"command":"echo","args":["--new"]},
        "skipme": {"command":"echo","args":["--new"]},
        "dup": {"command":"echo","args":["--duplicate"]},
        "same": {"command":"echo"},
        "keeps": {"command":"echo"},
        "broken": {"command":5},
        "remote": {"type":"http","url":"https://example.test/mcp","headers":{"X-Key":"oauth-secret-header"},"oauth":{"clientId":"client","scopes":"a b","extraSetting":true}},
        "authed": {"type":"http","url":"https://example.test/mcp","auth":{"type":"bearer","token":"oauth-secret-token","rotate":"daily"}}
    }, "inputs": []});
    let before = user_revision(&mut client).await;
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"user","document":document.to_string()}),
        )
        .await;
    assert_ok(&preview);
    assert_eq!(preview["result"]["revision"], before);
    assert_eq!(preview["result"]["unknownTopLevel"], json!(["inputs"]));
    assert_eq!(entry(&preview, "fresh")["status"], "changed");
    assert_eq!(entry(&preview, "fresh")["transport"], "stdio");
    assert_eq!(entry(&preview, "github")["status"], "conflict");
    assert_eq!(entry(&preview, "skipme")["status"], "conflict");
    assert_eq!(entry(&preview, "dup")["status"], "conflict");
    assert_eq!(entry(&preview, "same")["status"], "identical");
    assert_eq!(entry(&preview, "keeps")["status"], "identical");
    assert_eq!(
        entry(&preview, "keeps")["changes"],
        json!([{"field": "enabled", "kind": "boolean"}]),
        "an enabled entry names the disablement a replace would write"
    );
    assert_eq!(
        entry(&preview, "same")["changes"],
        json!([]),
        "a disabled entry's replace changes no field"
    );
    assert_eq!(entry(&preview, "broken")["status"], "invalid");
    assert_eq!(entry(&preview, "remote")["status"], "changed");
    assert_eq!(entry(&preview, "remote")["transport"], "http");
    assert_eq!(entry(&preview, "authed")["status"], "changed");
    assert_eq!(fields(entry(&preview, "github")), ["args", "enabled"]);
    assert_eq!(
        fields(entry(&preview, "fresh")),
        ["args", "command", "enabled", "env", "extensionField"]
    );
    assert_eq!(
        unsupported(entry(&preview, "fresh")),
        [("extensionField", "kept-unused")]
    );
    assert_eq!(
        unsupported(entry(&preview, "remote")),
        [("oauth.extraSetting", "kept-unused")]
    );
    assert_eq!(
        unsupported(entry(&preview, "authed")),
        [("auth.rotate", "dropped")]
    );
    assert!(
        !preview.to_string().contains("oauth-secret"),
        "a preview carries field names and kinds, never values: {preview}"
    );
    assert_eq!(user_revision(&mut client).await, before);
    let applied = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"user",
                "document":document.to_string(),
                "expectedVersion":preview["result"]["revision"],
                "resolutions":[
                    {"id":"fresh","action":"import"},
                    {"id":"github","action":"replace"},
                    {"id":"skipme","action":"skip"},
                    {"id":"dup","action":"rename","newId":"dup-copy"},
                    {"id":"same","action":"import"},
                    {"id":"keeps","action":"import"},
                    {"id":"broken","action":"skip"},
                    {"id":"remote","action":"import"},
                    {"id":"authed","action":"import"}
                ]
            }),
        )
        .await;
    assert_ok(&applied);
    assert_ne!(applied["result"]["revision"], before);
    let catalog = client.rpc("mcp.catalog", json!({})).await;
    assert_ok(&catalog);
    assert_eq!(catalog["result"]["servers"].as_array().unwrap().len(), 9);
    assert_eq!(catalog_entry(&catalog, "fresh")["enabled"], false);
    assert_eq!(catalog_entry(&catalog, "fresh")["transport"], "stdio");
    assert_eq!(catalog_entry(&catalog, "authed")["enabled"], false);
    assert_eq!(catalog_entry(&catalog, "github")["enabled"], false);
    assert_eq!(
        catalog_entry(&catalog, "keeps")["enabled"],
        true,
        "an identical import leaves the target's enablement as it was"
    );
    let github = client
        .rpc("mcp.configuration", json!({"source":"user","id":"github"}))
        .await;
    assert_ok(&github);
    assert_eq!(github["result"]["configuration"]["argsCount"], 1);
    assert_eq!(github["result"]["enabled"], false);
    let skipped = client
        .rpc("mcp.configuration", json!({"source":"user","id":"skipme"}))
        .await;
    assert_ok(&skipped);
    assert_eq!(
        skipped["result"]["configuration"]["argsCount"], 2,
        "a skipped conflict keeps the target entry as written"
    );
    let renamed = client
        .rpc(
            "mcp.configuration",
            json!({"source":"user","id":"dup-copy"}),
        )
        .await;
    assert_ok(&renamed);
    assert_eq!(renamed["result"]["configuration"]["argsCount"], 1);
    let untouched = client
        .rpc("mcp.configuration", json!({"source":"user","id":"dup"}))
        .await;
    assert_ok(&untouched);
    assert_eq!(untouched["result"]["configuration"]["argsCount"], 2);
    assert_eq!(
        client
            .rpc("mcp.configuration", json!({"source":"user","id":"broken"}))
            .await["error"]["data"]["appError"],
        "mcp_unknown_server"
    );

    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"user","document":document.to_string()}),
        )
        .await;
    assert_ok(&preview);
    for id in ["fresh", "github", "same", "keeps", "remote", "authed"] {
        assert_eq!(entry(&preview, id)["status"], "identical", "{id}");
    }
    let revision = preview["result"]["revision"].clone();
    let unchanged = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"user",
                "document":document.to_string(),
                "expectedVersion":revision,
                "resolutions":[]
            }),
        )
        .await;
    assert_ok(&unchanged);
    assert_eq!(
        unchanged["result"]["revision"], revision,
        "a re-import that accepts nothing writes nothing"
    );
    let after = client.rpc("mcp.catalog", json!({})).await;
    assert_eq!(after["result"]["servers"].as_array().unwrap().len(), 9);
    assert_eq!(after["result"]["userRevision"], revision);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

const ENV_VALUE: &str = "fixture-environment-value";
const ARG_VALUE: &str = "--token=fixture-argument";
const HEADER_VALUE: &str = "fixture-header-value";
const BEARER_VALUE: &str = "fixture-bearer-value";
const QUERY_KEY: &str = "fixture-query-value";
const PASSWORD_VALUE: &str = "fixture-userinfo-password";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_writes_project_entries_through_the_withheld_record() {
    let home = scratch("dm-import-project");
    let project = home.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join(".mcp.json"),
        json!({"mcpServers":{"broken":{"command":5}}}).to_string(),
    )
    .unwrap();
    let project = std::fs::canonicalize(&project).unwrap();
    let (server, _) = launch_with_agent(
        Arc::new(MockTools),
        ServerConfig {
            product_home: Some(home.clone()),
            target_home: Some(home.join("native")),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let query_url = format!("https://example.test/mcp?key={QUERY_KEY}");
    let document = json!({"mcpServers": {
        "plain": {"command":"echo"},
        "envlit": {"command":"echo","env":{"TOKEN":ENV_VALUE}},
        "envdefault": {"command":"echo","env":{"TOKEN":"${TOKEN:-sk-live}"}},
        "argslit": {"command":"echo","args":[ARG_VALUE]},
        "headerlit": {"type":"http","url":"https://example.test/mcp","headers":{"X-Key":HEADER_VALUE}},
        "querylit": {"type":"http","url":query_url.clone()},
        "refonly": {"type":"http","url":"https://example.test/mcp","headers":{"Authorization":"${AUTH}"}},
        "tokenlit": {"type":"http","url":"https://example.test/mcp","auth":{"type":"bearer","token":BEARER_VALUE}},
        "tokendefault": {"type":"http","url":"https://example.test/mcp","auth":{"type":"bearer","token":"${TOKEN:-sk-live}"}},
        "userinfo": {"type":"http","url":format!("https://fixture-user:{PASSWORD_VALUE}@example.test/mcp")}
    }})
    .to_string();
    let ids = [
        "plain",
        "envlit",
        "envdefault",
        "argslit",
        "headerlit",
        "querylit",
        "refonly",
        "tokenlit",
        "tokendefault",
        "userinfo",
    ];
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":document}),
        )
        .await;
    assert_ok(&preview);
    for id in ids {
        assert_eq!(entry(&preview, id)["status"], "changed", "{id}");
        assert!(entry(&preview, id)["error"].is_null(), "{id}");
        assert!(
            !entry(&preview, id)
                .as_object()
                .unwrap()
                .contains_key("confirm"),
            "the project contract takes no per-entry confirmation: {id}"
        );
    }
    for secret in [
        ENV_VALUE,
        ARG_VALUE,
        HEADER_VALUE,
        BEARER_VALUE,
        QUERY_KEY,
        PASSWORD_VALUE,
        "sk-live",
    ] {
        assert!(
            !preview.to_string().contains(secret),
            "a preview carries field names and kinds, never values: {preview}"
        );
    }
    let applied = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":document,
                "expectedVersion":preview["result"]["revision"],
                "resolutions": ids.map(|id| json!({"id":id,"action":"import"}))
            }),
        )
        .await;
    assert_ok(&applied);
    let stored: Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    let servers = &stored["mcpServers"];
    assert_eq!(servers["broken"], json!({"command":5}));
    for id in ["plain", "envlit", "envdefault", "argslit"] {
        assert_eq!(servers[id], json!({"command":"echo"}), "{id}");
    }
    for id in [
        "headerlit",
        "querylit",
        "refonly",
        "tokenlit",
        "tokendefault",
        "userinfo",
    ] {
        assert_eq!(
            servers[id],
            json!({"type":"http","url":"https://example.test"}),
            "{id}"
        );
    }
    let written = stored.to_string();
    for secret in [
        ENV_VALUE,
        ARG_VALUE,
        HEADER_VALUE,
        BEARER_VALUE,
        QUERY_KEY,
        PASSWORD_VALUE,
        "sk-live",
        "${AUTH}",
    ] {
        assert!(
            !written.contains(secret),
            "the workspace file never receives {secret}: {stored}"
        );
    }
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    for id in ids {
        assert_eq!(catalog_entry(&catalog, id)["enabled"], false, "{id}");
    }
    // The record carries every withheld value, references and literal
    // defaults included: the source reads back as the document's
    // configuration with no field left to change.
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":document}),
        )
        .await;
    assert_ok(&preview);
    for id in ids {
        assert_eq!(entry(&preview, id)["status"], "identical", "{id}");
        assert!(fields(entry(&preview, id)).is_empty(), "{id}");
    }
    // An identical entry is replaceable and renamable: the split writes the
    // same pair, and the destination gets its own record.
    let revision = preview["result"]["revision"].clone();
    let rewritten = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":document,
                "expectedVersion":revision,
                "resolutions":[
                    {"id":"envlit","action":"replace"},
                    {"id":"tokenlit","action":"rename","newId":"tokenlit-copy"}
                ]
            }),
        )
        .await;
    assert_ok(&rewritten);
    let stored: Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(
        stored["mcpServers"]["envlit"],
        json!({"command":"echo"}),
        "a replace of an identical entry keeps the values"
    );
    assert_eq!(
        stored["mcpServers"]["tokenlit-copy"],
        json!({"type":"http","url":"https://example.test"}),
        "a rename writes the skeleton under the new id"
    );
    assert_eq!(
        stored["mcpServers"]["tokenlit"],
        json!({"type":"http","url":"https://example.test"}),
        "a rename leaves the source entry as it was"
    );
    let copy = client
        .rpc(
            "mcp.configuration",
            json!({"source":"project","id":"tokenlit-copy","cwd":project}),
        )
        .await;
    assert_ok(&copy);
    assert_eq!(copy["result"]["secretsInUserScope"], true);
    assert_eq!(copy["result"]["configuration"]["authConfigured"], true);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_replaces_a_withheld_record_without_an_entry() {
    let home = scratch("dm-import-withheld");
    let project = home.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join(".mcp.json"),
        json!({"mcpServers":{}}).to_string(),
    )
    .unwrap();
    let project = std::fs::canonicalize(&project).unwrap();
    let (server, _) = launch_with_agent(
        Arc::new(MockTools),
        ServerConfig {
            product_home: Some(home.clone()),
            target_home: Some(home.join("native")),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    // `kept` keeps its args in `mcp-secrets`; `plain` leaves no record.
    move_to_project(
        &mut client,
        &project,
        "kept",
        json!({"command":"echo","args":["--withheld"]}),
    )
    .await;
    move_to_project(&mut client, &project, "plain", json!({"command":"echo"})).await;
    for id in ["kept", "plain"] {
        let revision = project_revision(&mut client, &project).await;
        assert_ok(&client.rpc("mcp.set_enabled", json!({"source":"project","id":id,"cwd":project,"expectedVersion":revision,"enabled":true})).await);
    }
    // An external edit removes both entries; their records stay.
    std::fs::write(
        project.join(".mcp.json"),
        json!({"mcpServers":{}}).to_string(),
    )
    .unwrap();
    let document = servers_document(json!({
        "kept": {"command":"echo"},
        "plain": {"command":"echo"}
    }));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":document}),
        )
        .await;
    assert_ok(&preview);
    for id in ["kept", "plain"] {
        assert_eq!(entry(&preview, id)["status"], "changed", "{id}");
    }
    // The import writes the destination's record itself, so a leftover
    // record neither refuses the import nor merges into it: an entry with no
    // withheld part removes the record.
    let revision = project_revision(&mut client, &project).await;
    assert_ok(
        &client
            .rpc(
                "mcp.import.apply",
                json!({
                    "source":"project",
                    "cwd":project,
                    "document":document,
                    "expectedVersion":revision,
                    "resolutions":[{"id":"kept","action":"import"}]
                }),
            )
            .await,
    );
    let kept = client
        .rpc(
            "mcp.configuration",
            json!({"source":"project","id":"kept","cwd":project}),
        )
        .await;
    assert_ok(&kept);
    assert_eq!(
        kept["result"]["secretsInUserScope"], false,
        "the leftover record was replaced by what the import carried"
    );
    assert_eq!(kept["result"]["configuration"]["argsCount"], 0);
    // The stale enablement record of `plain` is withdrawn before the write,
    // so the entry the import writes starts disabled (D24).
    let revision = project_revision(&mut client, &project).await;
    assert_ok(
        &client
            .rpc(
                "mcp.import.apply",
                json!({
                    "source":"project",
                    "cwd":project,
                    "document":document,
                    "expectedVersion":revision,
                    "resolutions":[{"id":"plain","action":"import"}]
                }),
            )
            .await,
    );
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    assert_eq!(
        catalog_entry(&catalog, "plain")["enabled"],
        false,
        "an import never enables anything"
    );
    let stored: Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(stored["mcpServers"]["kept"], json!({"command":"echo"}));
    assert_eq!(stored["mcpServers"]["plain"], json!({"command":"echo"}));
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

async fn project_revision(client: &mut Client, project: &std::path::Path) -> Value {
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    catalog["result"]["projectRevision"].clone()
}

/// Saves an entry in user scope and moves it into the project source, where a
/// literal value becomes a withheld `mcp-secrets` record.
async fn move_to_project(
    client: &mut Client,
    project: &std::path::Path,
    id: &str,
    configuration: Value,
) {
    let user = client.rpc("mcp.catalog", json!({})).await;
    assert_ok(
        &client
            .rpc(
                "mcp.save",
                json!({"source":"user","id":id,"expectedVersion":user["result"]["userRevision"],"configuration":configuration}),
            )
            .await,
    );
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&client.rpc("mcp.move", json!({"id":id,"from":"user","to":"project","cwd":project,"expectedFromVersion":catalog["result"]["userRevision"],"expectedToVersion":catalog["result"]["projectRevision"]})).await);
}

/// The reply reports a bounded number of names per list and counts the rest,
/// so a document of long unknown field names cannot push the reply past the
/// transport frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_bounds_the_field_names_one_entry_reports() {
    const MAX: usize = 32;
    const UNKNOWN: usize = 40;
    let home = scratch("dm-import-cap");
    let (server, _) = launch_with_agent(
        Arc::new(MockTools),
        ServerConfig {
            product_home: Some(home.clone()),
            target_home: Some(home.join("native")),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let mut wide = serde_json::Map::new();
    wide.insert("command".into(), json!("echo"));
    for index in 0..UNKNOWN {
        wide.insert(format!("unknown{index:02}"), json!(index));
    }
    let document = servers_document(json!({"wide": wide}));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"user","document":document}),
        )
        .await;
    assert_ok(&preview);
    let wide = entry(&preview, "wide");
    assert_eq!(wide["status"], "changed");
    // `changes` compares against the free id and names the entry's own
    // `command` and `enabled` members too; `unsupported` names only the
    // members the decoder does not act on.
    assert_eq!(wide["changes"].as_array().unwrap().len(), MAX);
    assert_eq!(wide["unsupported"].as_array().unwrap().len(), MAX);
    assert_eq!(
        wide["omitted"],
        json!((UNKNOWN + 2 - MAX) + (UNKNOWN - MAX)),
        "the count matches the names the cap left out"
    );
    assert_eq!(wide["changes"][0]["field"], "command");
    assert_eq!(wide["changes"][1]["field"], "enabled");
    assert_eq!(wide["unsupported"][0]["field"], "unknown00");
    assert_eq!(wide["unsupported"][0]["effect"], "kept-unused");
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_answers_stale_revisions_and_refuses_oversize_documents() {
    let home = scratch("dm-import-limits");
    let (server, _) = launch_with_agent(
        Arc::new(MockTools),
        ServerConfig {
            product_home: Some(home.clone()),
            target_home: Some(home.join("native")),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let small = servers_document(json!({"a": {"command":"echo"}}));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"user","document":small}),
        )
        .await;
    assert_ok(&preview);
    let revision = preview["result"]["revision"].clone();
    save(&mut client, "b", json!({"command":"echo"})).await;
    let stale = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"user",
                "document":small,
                "expectedVersion":revision,
                "resolutions":[{"id":"a","action":"import"}]
            }),
        )
        .await;
    assert_eq!(stale["error"]["data"]["appError"], "mcp_settings_conflict");
    assert_eq!(
        client
            .rpc("mcp.configuration", json!({"source":"user","id":"a"}))
            .await["error"]["data"]["appError"],
        "mcp_unknown_server",
        "a stale apply writes nothing"
    );
    let malformed = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"user","document":"not json"}),
        )
        .await;
    assert_eq!(malformed["error"]["data"]["appError"], "mcp_invalid_config");
    assert!(
        malformed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("line")
    );
    let oversize =
        json!({"mcpServers":{"big":{"command":"echo","padding":"x".repeat(8 * 1024 * 1024)}}})
            .to_string();
    let refused = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"user","document":oversize}),
        )
        .await;
    assert_eq!(refused["error"]["data"]["appError"], "mcp_invalid_config");
    let mut servers = serde_json::Map::new();
    for index in 0..=1000 {
        servers.insert(format!("s{index}"), json!({"command":"echo"}));
    }
    let too_many = json!({"mcpServers":servers}).to_string();
    let refused = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"user","document":too_many}),
        )
        .await;
    assert_eq!(refused["error"]["data"]["appError"], "mcp_invalid_config");
    let revision = user_revision(&mut client).await;
    let unknown = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"user",
                "document":small,
                "expectedVersion":revision,
                "resolutions":[{"id":"missing","action":"import"}]
            }),
        )
        .await;
    assert_eq!(unknown["error"]["code"], -32602);
    let revision = user_revision(&mut client).await;
    let stray_rename = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"user",
                "document":small,
                "expectedVersion":revision,
                "resolutions":[{"id":"a","action":"import","newId":"other"}]
            }),
        )
        .await;
    assert_eq!(stray_rename["error"]["code"], -32602);
    let project = home.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let project = std::fs::canonicalize(&project).unwrap();
    let project_document = servers_document(json!({"plain": {"command":"echo"}}));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":project_document}),
        )
        .await;
    assert_ok(&preview);
    assert_eq!(
        preview["result"]["revision"], "missing",
        "a workspace without .mcp.json reports the catalog's missing revision"
    );
    let applied = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":project_document,
                "expectedVersion":"missing",
                "resolutions":[{"id":"plain","action":"import"}]
            }),
        )
        .await;
    assert_ok(&applied);
    assert_ne!(applied["result"]["revision"], "missing");
    let stored: Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(stored["mcpServers"]["plain"], json!({"command":"echo"}));
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}
