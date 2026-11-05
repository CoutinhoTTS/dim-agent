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
        ["args", "command", "enabled", "env"],
        "an extension member is named by unsupported, not by changes"
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_replaces_a_project_entry_and_starts_it_disabled() {
    let home = scratch("dm-import-replace-enabled");
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
    let first = servers_document(json!({"local": {"command":"echo","args":["--one"]}}));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":first}),
        )
        .await;
    assert_ok(&preview);
    assert_ok(
        &client
            .rpc(
                "mcp.import.apply",
                json!({
                    "source":"project",
                    "cwd":project,
                    "document":first,
                    "expectedVersion":preview["result"]["revision"],
                    "resolutions":[{"id":"local","action":"import"}]
                }),
            )
            .await,
    );
    let revision = project_revision(&mut client, &project).await;
    assert_ok(&client.rpc("mcp.set_enabled", json!({"source":"project","id":"local","cwd":project,"expectedVersion":revision,"enabled":true})).await);
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    assert_eq!(catalog_entry(&catalog, "local")["enabled"], true);
    let second = servers_document(json!({"local": {"command":"echo","args":["--two"]}}));
    let revision = project_revision(&mut client, &project).await;
    let conflicted = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":second,
                "expectedVersion":revision,
                "resolutions":[{"id":"local","action":"import"}]
            }),
        )
        .await;
    assert_eq!(
        conflicted["error"]["data"]["appError"], "mcp_import_conflict",
        "a changed entry is replaced explicitly, never imported"
    );
    let replaced = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":second,
                "expectedVersion":revision,
                "resolutions":[{"id":"local","action":"replace"}]
            }),
        )
        .await;
    assert_ok(&replaced);
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    assert_eq!(
        catalog_entry(&catalog, "local")["enabled"],
        false,
        "a replace starts disabled like an import (D24)"
    );
    let local = client
        .rpc(
            "mcp.configuration",
            json!({"source":"project","id":"local","cwd":project}),
        )
        .await;
    assert_ok(&local);
    assert_eq!(local["result"]["secretsInUserScope"], true);
    assert_eq!(local["result"]["configuration"]["argsCount"], 1);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

async fn project_revision(client: &mut Client, project: &std::path::Path) -> Value {
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    catalog["result"]["projectRevision"].clone()
}

/// A source compare-and-set that fails writes no entry, but it withdraws
/// enablement before the source write, so the entry it would have replaced
/// stays disabled: the fail-closed direction, which `mcp.set_enabled`
/// reverses. The source keeps the winner's write and nothing of the loser's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_that_loses_the_source_cas_leaves_the_entry_disabled() {
    let home = scratch("dm-import-cas");
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
    let first = servers_document(json!({
        "alpha": {"command":"echo","args":["--one"]},
        "beta": {"command":"echo","args":["--two"]}
    }));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":first}),
        )
        .await;
    assert_ok(&preview);
    assert_ok(
        &client
            .rpc(
                "mcp.import.apply",
                json!({
                    "source":"project",
                    "cwd":project,
                    "document":first,
                    "expectedVersion":preview["result"]["revision"],
                    "resolutions":[
                        {"id":"alpha","action":"import"},
                        {"id":"beta","action":"import"}
                    ]
                }),
            )
            .await,
    );
    let revision = project_revision(&mut client, &project).await;
    for id in ["alpha", "beta"] {
        assert_ok(
            &client
                .rpc(
                    "mcp.set_enabled",
                    json!({"source":"project","id":id,"cwd":project,"expectedVersion":revision,"enabled":true}),
                )
                .await,
        );
    }
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    assert_eq!(catalog_entry(&catalog, "alpha")["enabled"], true);
    assert_eq!(catalog_entry(&catalog, "beta")["enabled"], true);
    let revision = catalog["result"]["projectRevision"].clone();
    // Each document changes its entry's public skeleton, so the source
    // revision moves: a replace that only rewrote withheld values would
    // leave the bytes equal and the loser's compare-and-set would pass.
    let alpha = servers_document(json!({"alpha": {"command":"alpha-two","args":["--three"]}}));
    let beta = servers_document(json!({"beta": {"command":"beta-two","args":["--four"]}}));
    let alpha_id = client
        .send(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":alpha,
                "expectedVersion":revision,
                "resolutions":[{"id":"alpha","action":"replace"}]
            }),
        )
        .await;
    let beta_id = client
        .send(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":beta,
                "expectedVersion":revision,
                "resolutions":[{"id":"beta","action":"replace"}]
            }),
        )
        .await;
    let alpha_result = client.response(alpha_id).await;
    let beta_result = client.response(beta_id).await;
    assert!(
        alpha_result.get("error").is_none() != beta_result.get("error").is_none(),
        "one import wins the source revision: {alpha_result} {beta_result}"
    );
    let (winner, loser, loser_result) = if alpha_result.get("error").is_none() {
        ("alpha", "beta", &beta_result)
    } else {
        ("beta", "alpha", &alpha_result)
    };
    assert_eq!(
        loser_result["error"]["data"]["appError"], "mcp_settings_conflict",
        "the import that lost the write answers a conflict: {loser_result}"
    );
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    assert_eq!(
        catalog_entry(&catalog, winner)["enabled"],
        false,
        "the import that committed starts its entry disabled (D24)"
    );
    assert_eq!(
        catalog_entry(&catalog, loser)["enabled"],
        false,
        "a failed source write leaves the entry it would have replaced disabled"
    );
    let stored: Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    let (winner_command, loser_command) = if winner == "alpha" {
        ("alpha-two", "beta-two")
    } else {
        ("beta-two", "alpha-two")
    };
    assert_eq!(
        stored["mcpServers"][winner]["command"], winner_command,
        "the source holds the winner's write: {stored}"
    );
    assert_eq!(
        stored["mcpServers"][loser]["command"], "echo",
        "the source holds nothing of the loser's write: {stored}"
    );
    assert!(
        !stored.to_string().contains(loser_command),
        "the loser's write never enters the source: {stored}"
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

/// A record write that fails after the source commits answers
/// `mcp_import_partial`: the entries stay written and disabled with no
/// withheld value, and replacing them again once the store accepts writes
/// rewrites both stores.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_answers_partial_when_the_record_write_fails() {
    let home = scratch("dm-import-partial");
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
    let document = servers_document(json!({
        "held": {"command":"echo","args":[ARG_VALUE]},
        "plain": {"command":"echo"}
    }));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":document}),
        )
        .await;
    assert_ok(&preview);
    // The native store holds the user documents. A trigger that refuses
    // updates to the withheld-record row leaves every read working — the
    // preview above read the store — and fails the withheld-record write
    // alone; the enablement withdrawal before the source write is not it.
    let native = std::fs::canonicalize(home.join("native")).unwrap();
    let database = rusqlite::Connection::open(native.join("dimcode.sqlite")).unwrap();
    database
        .busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    database
        .execute_batch(
            "CREATE TRIGGER import_holds_records BEFORE UPDATE ON settings_documents
             WHEN OLD.source = 'user/mcp-secrets'
             BEGIN SELECT RAISE(ABORT, 'held for the test'); END;",
        )
        .unwrap();
    let applied = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":document,
                "expectedVersion":preview["result"]["revision"],
                "resolutions":[
                    {"id":"held","action":"import"},
                    {"id":"plain","action":"import"}
                ]
            }),
        )
        .await;
    database
        .execute_batch("DROP TRIGGER import_holds_records")
        .unwrap();
    assert_eq!(
        applied["error"]["data"]["appError"], "mcp_import_partial",
        "the source write committed and the record write did not: {applied}"
    );
    let stored: Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    for id in ["held", "plain"] {
        assert_eq!(
            stored["mcpServers"][id],
            json!({"command":"echo"}),
            "the entry is written as its public skeleton: {id}"
        );
    }
    assert!(
        !stored.to_string().contains(ARG_VALUE),
        "the withheld value never enters the workspace file: {stored}"
    );
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    for id in ["held", "plain"] {
        assert_eq!(catalog_entry(&catalog, id)["enabled"], false, "{id}");
    }
    // Restoring the store and replacing the entries writes both stores.
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":document}),
        )
        .await;
    assert_ok(&preview);
    assert_eq!(
        entry(&preview, "held")["status"],
        "conflict",
        "the written skeleton does not carry the document's withheld value"
    );
    assert_ok(
        &client
            .rpc(
                "mcp.import.apply",
                json!({
                    "source":"project",
                    "cwd":project,
                    "document":document,
                    "expectedVersion":preview["result"]["revision"],
                    "resolutions":[
                        {"id":"held","action":"replace"},
                        {"id":"plain","action":"replace"}
                    ]
                }),
            )
            .await,
    );
    let stored: String = database
        .query_row(
            "SELECT document FROM settings_documents WHERE source='user/mcp-secrets'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let secrets: Value = serde_json::from_str(&stored).unwrap();
    let workspace = project.to_str().unwrap();
    assert_eq!(
        secrets["entries"][workspace]["held"]["args"],
        json!([ARG_VALUE]),
        "the replacement writes the withheld value"
    );
    assert!(
        secrets["entries"][workspace].get("plain").is_none(),
        "an entry without a withheld part leaves no record: {secrets}"
    );
    let stored: Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(stored["mcpServers"]["held"], json!({"command":"echo"}));
    assert!(!stored.to_string().contains(ARG_VALUE));
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    assert_eq!(catalog_entry(&catalog, "held")["enabled"], false);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

/// Extension members are not configuration: an entry whose declared members
/// match is `identical` whatever extension members either side holds, so an
/// import writes nothing, and a replace writes the document's entry as it
/// stands.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_ignores_extension_members_in_the_comparison() {
    let home = scratch("dm-import-extension");
    let project = home.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let stored = json!({"mcpServers":{
        "kept": {"command":"echo","x-note":"a"},
        "gained": {"command":"echo"}
    }});
    std::fs::write(project.join(".mcp.json"), stored.to_string()).unwrap();
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
    let document = servers_document(json!({
        "kept": {"command":"echo"},
        "gained": {"command":"echo","x-note":"b"}
    }));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":document}),
        )
        .await;
    assert_ok(&preview);
    // The target's extension member is not compared, and the document's is
    // reported as unsupported rather than as a change.
    let kept = entry(&preview, "kept");
    assert_eq!(kept["status"], "identical");
    assert_eq!(fields(kept), Vec::<&str>::new());
    assert_eq!(unsupported(kept), Vec::new());
    let gained = entry(&preview, "gained");
    assert_eq!(gained["status"], "identical");
    assert_eq!(fields(gained), Vec::<&str>::new());
    assert_eq!(unsupported(gained), [("x-note", "kept-unused")]);
    // An import of identical entries writes nothing.
    let before = std::fs::read(project.join(".mcp.json")).unwrap();
    assert_ok(
        &client
            .rpc(
                "mcp.import.apply",
                json!({
                    "source":"project",
                    "cwd":project,
                    "document":document,
                    "expectedVersion":preview["result"]["revision"],
                    "resolutions":[
                        {"id":"kept","action":"import"},
                        {"id":"gained","action":"import"}
                    ]
                }),
            )
            .await,
    );
    assert_eq!(
        std::fs::read(project.join(".mcp.json")).unwrap(),
        before,
        "an import of identical entries leaves the source untouched"
    );
    // A replace writes the document's entry as it stands: the extension
    // member only the target held is not kept.
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":document}),
        )
        .await;
    assert_ok(&preview);
    assert_ok(
        &client
            .rpc(
                "mcp.import.apply",
                json!({
                    "source":"project",
                    "cwd":project,
                    "document":document,
                    "expectedVersion":preview["result"]["revision"],
                    "resolutions":[{"id":"kept","action":"replace"}]
                }),
            )
            .await,
    );
    let stored: Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(
        stored["mcpServers"]["kept"],
        json!({"command":"echo"}),
        "the replace writes the document's entry: {stored}"
    );
    assert_eq!(
        stored["mcpServers"]["gained"],
        json!({"command":"echo"}),
        "an untouched entry stays as written: {stored}"
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

/// `oauth`'s unknown names are extension members too: a target that carries
/// one and a document that does not compare `identical`, the document's are
/// named `oauth.<name>`, and an import writes neither store; a declared
/// `oauth` difference is still a `conflict`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_ignores_oauth_extension_members_in_the_comparison() {
    let home = scratch("dm-import-oauth-extension");
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
    // Import the entries the comparison reads; a project import keeps the
    // `kept` entry's oauth extension member in the withheld record.
    let seeded = servers_document(json!({
        "kept": {"type":"http","url":"https://example.test/mcp","oauth":{"clientId":"x","x-note":"a"}},
        "gained": {"type":"http","url":"https://example.test/mcp","oauth":{"clientId":"x"}},
        "changed": {"type":"http","url":"https://example.test/mcp","oauth":{"clientId":"old"}}
    }));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":seeded}),
        )
        .await;
    assert_ok(&preview);
    assert_ok(
        &client
            .rpc(
                "mcp.import.apply",
                json!({
                    "source":"project",
                    "cwd":project,
                    "document":seeded,
                    "expectedVersion":preview["result"]["revision"],
                    "resolutions":[
                        {"id":"kept","action":"import"},
                        {"id":"gained","action":"import"},
                        {"id":"changed","action":"import"}
                    ]
                }),
            )
            .await,
    );
    let document = servers_document(json!({
        "kept": {"type":"http","url":"https://example.test/mcp","oauth":{"clientId":"x"}},
        "gained": {"type":"http","url":"https://example.test/mcp","oauth":{"clientId":"x","x-note":"b"}},
        "changed": {"type":"http","url":"https://example.test/mcp","oauth":{"clientId":"new"}}
    }));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":document}),
        )
        .await;
    assert_ok(&preview);
    // The target-only oauth extension member is not compared.
    let kept = entry(&preview, "kept");
    assert_eq!(kept["status"], "identical");
    assert_eq!(fields(kept), Vec::<&str>::new());
    assert_eq!(unsupported(kept), Vec::new());
    // The document-only one is unsupported, not a change.
    let gained = entry(&preview, "gained");
    assert_eq!(gained["status"], "identical");
    assert_eq!(fields(gained), Vec::<&str>::new());
    assert_eq!(unsupported(gained), [("oauth.x-note", "kept-unused")]);
    // A declared oauth difference is still a conflict.
    let changed = entry(&preview, "changed");
    assert_eq!(changed["status"], "conflict");
    assert_eq!(fields(changed), ["oauth"]);
    // The import writes neither store.
    fn records(database: &rusqlite::Connection) -> Option<String> {
        database
            .query_row(
                "SELECT document FROM settings_documents WHERE source='user/mcp-secrets'",
                [],
                |row| row.get(0),
            )
            .ok()
    }
    let before = std::fs::read(project.join(".mcp.json")).unwrap();
    let native = std::fs::canonicalize(home.join("native")).unwrap();
    let database = rusqlite::Connection::open(native.join("dimcode.sqlite")).unwrap();
    database
        .busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let records_before = records(&database);
    assert!(
        records_before
            .as_deref()
            .is_some_and(|records| records.contains("x-note")),
        "the seeded import wrote the oauth extension member to the record: {records_before:?}"
    );
    assert_ok(
        &client
            .rpc(
                "mcp.import.apply",
                json!({
                    "source":"project",
                    "cwd":project,
                    "document":document,
                    "expectedVersion":preview["result"]["revision"],
                    "resolutions":[
                        {"id":"kept","action":"import"},
                        {"id":"gained","action":"import"}
                    ]
                }),
            )
            .await,
    );
    assert_eq!(
        std::fs::read(project.join(".mcp.json")).unwrap(),
        before,
        "the import leaves the source untouched"
    );
    assert_eq!(
        records(&database),
        records_before,
        "the import leaves the record untouched"
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
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
    // `changes` compares against the free id and names only the entry's
    // declared `command` and `enabled` members; `unsupported` names the
    // members the decoder does not act on.
    assert_eq!(fields(wide), ["command", "enabled"]);
    assert_eq!(unsupported(wide).len(), MAX);
    assert_eq!(
        wide["omitted"],
        json!(UNKNOWN - MAX),
        "only the unknown names the cap left out are missing"
    );
    assert_eq!(wide["unsupported"][0]["field"], "unknown00");
    assert_eq!(wide["unsupported"][0]["effect"], "kept-unused");
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

/// A document just under the size cap whose entry carries long unknown member
/// names still produces a reply: each name is reported once, so the reply
/// stays under the transport frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_answers_a_document_of_long_field_names() {
    const LONG: usize = 253_000;
    const UNKNOWN: usize = 33;
    const MAX: usize = 32;
    let home = scratch("dm-import-long-names");
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
        wide.insert(format!("{index:02}{}", "x".repeat(LONG)), json!(index));
    }
    let document = servers_document(json!({"wide": wide}));
    assert!(
        document.len() < 8 * 1024 * 1024,
        "the document is under the size cap: {}",
        document.len()
    );
    assert!(
        document.len() > 8 * 1024 * 1024 - 2 * LONG,
        "the document is just under the size cap: {}",
        document.len()
    );
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"user","document":document}),
        )
        .await;
    assert_ok(&preview);
    let wide = entry(&preview, "wide");
    assert_eq!(wide["status"], "changed");
    assert_eq!(fields(wide), ["command", "enabled"]);
    assert_eq!(unsupported(wide).len(), MAX);
    assert_eq!(wide["omitted"], json!(UNKNOWN - MAX));
    assert!(
        preview.to_string().len() < 16 * 1024 * 1024,
        "the reply stays under the transport frame: {}",
        preview.to_string().len()
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

/// A document just under the size cap whose entries do not decode still
/// produces a reply: each entry's long id is reported once, in `id`, so the
/// reply stays under the transport frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_answers_a_document_of_long_ids() {
    const ID: usize = 8_378;
    const ENTRIES: usize = 1_000;
    let home = scratch("dm-import-long-ids");
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
    let mut servers = serde_json::Map::new();
    for index in 0..ENTRIES {
        // A non-object value does not decode; the key is the entry's id.
        servers.insert(format!("{index:04}{}", "a".repeat(ID - 4)), json!(index));
    }
    let document = json!({"mcpServers": servers}).to_string();
    assert!(
        document.len() < 8 * 1024 * 1024,
        "the document is under the size cap: {}",
        document.len()
    );
    assert!(
        document.len() > 8 * 1024 * 1024 - 2 * ID,
        "the document is just under the size cap: {}",
        document.len()
    );
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"user","document":document}),
        )
        .await;
    assert_ok(&preview);
    let entries = preview["result"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), ENTRIES);
    for entry in entries {
        assert_eq!(entry["status"], "invalid");
        let id = entry["id"].as_str().unwrap();
        let error = entry["error"].as_str().unwrap();
        assert!(
            !error.contains(id),
            "the error does not repeat the entry's id: {error}"
        );
    }
    assert!(
        preview.to_string().len() < 16 * 1024 * 1024,
        "the reply stays under the transport frame: {}",
        preview.to_string().len()
    );
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
