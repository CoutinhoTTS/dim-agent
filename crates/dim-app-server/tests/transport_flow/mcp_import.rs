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
    assert_eq!(entry(&preview, "keeps")["changes"], json!([]));
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
    assert_eq!(entry(&preview, "fresh")["confirm"], json!([]));
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
async fn mcp_import_refuses_project_credentials_and_requires_literal_confirmation() {
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
        "argslit": {"command":"echo","args":[ARG_VALUE]},
        "headerlit": {"type":"http","url":"https://example.test/mcp","headers":{"X-Key":HEADER_VALUE}},
        "querylit": {"type":"http","url":query_url.clone()},
        "refonly": {"type":"http","url":"https://example.test/mcp","headers":{"Authorization":"${AUTH}"}},
        "tokenlit": {"type":"http","url":"https://example.test/mcp","auth":{"type":"bearer","token":BEARER_VALUE}},
        "userinfo": {"type":"http","url":format!("https://fixture-user:{PASSWORD_VALUE}@example.test/mcp")}
    }});
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":document.to_string()}),
        )
        .await;
    assert_ok(&preview);
    for (id, confirm) in [
        ("plain", json!([])),
        ("envlit", json!(["env"])),
        ("argslit", json!(["args"])),
        ("headerlit", json!(["headers"])),
        ("querylit", json!(["url"])),
        ("refonly", json!([])),
    ] {
        assert_eq!(entry(&preview, id)["status"], "changed", "{id}");
        assert_eq!(entry(&preview, id)["confirm"], confirm, "{id}");
    }
    assert_eq!(entry(&preview, "tokenlit")["status"], "invalid");
    assert!(
        entry(&preview, "tokenlit")["error"]
            .as_str()
            .unwrap()
            .contains("auth.token")
    );
    assert_eq!(entry(&preview, "userinfo")["status"], "invalid");
    assert!(
        entry(&preview, "userinfo")["error"]
            .as_str()
            .unwrap()
            .contains("userinfo")
    );
    for secret in [
        ENV_VALUE,
        ARG_VALUE,
        HEADER_VALUE,
        BEARER_VALUE,
        QUERY_KEY,
        PASSWORD_VALUE,
    ] {
        assert!(
            !preview.to_string().contains(secret),
            "a preview carries field names and kinds, never values: {preview}"
        );
    }
    let revision = preview["result"]["revision"].clone();
    let unconfirmed = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":document.to_string(),
                "expectedVersion":revision,
                "resolutions":[
                    {"id":"plain","action":"import"},
                    {"id":"envlit","action":"import"}
                ]
            }),
        )
        .await;
    assert_eq!(
        unconfirmed["error"]["data"]["appError"],
        "mcp_import_unconfirmed"
    );
    assert!(
        unconfirmed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("env")
    );
    assert_eq!(
        project_revision(&mut client, &project).await,
        revision,
        "an unconfirmed import writes nothing"
    );
    let applied = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":document.to_string(),
                "expectedVersion":revision,
                "resolutions":[
                    {"id":"plain","action":"import"},
                    {"id":"envlit","action":"import","confirm":["env"]},
                    {"id":"argslit","action":"import","confirm":["args"]},
                    {"id":"headerlit","action":"import","confirm":["headers"]},
                    {"id":"querylit","action":"import","confirm":["url"]},
                    {"id":"refonly","action":"import"}
                ]
            }),
        )
        .await;
    assert_ok(&applied);
    let stored: Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    let servers = &stored["mcpServers"];
    assert_eq!(servers["broken"], json!({"command":5}));
    for id in [
        "plain",
        "envlit",
        "argslit",
        "headerlit",
        "querylit",
        "refonly",
    ] {
        assert!(
            servers[id].get("enabled").is_none(),
            "a project import writes no enablement: {id}"
        );
    }
    assert_eq!(servers["envlit"]["env"]["TOKEN"].as_str(), Some(ENV_VALUE));
    assert_eq!(servers["argslit"]["args"][0].as_str(), Some(ARG_VALUE));
    assert_eq!(
        servers["headerlit"]["headers"]["X-Key"].as_str(),
        Some(HEADER_VALUE)
    );
    assert_eq!(
        servers["querylit"]["url"].as_str(),
        Some(query_url.as_str())
    );
    assert_eq!(servers["refonly"]["headers"]["Authorization"], "${AUTH}");
    assert!(servers.get("tokenlit").is_none());
    assert!(servers.get("userinfo").is_none());
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    for id in ["plain", "envlit", "argslit", "headerlit", "querylit"] {
        assert_eq!(catalog_entry(&catalog, id)["enabled"], false, "{id}");
    }
    let revision = project_revision(&mut client, &project).await;
    let refused = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":document.to_string(),
                "expectedVersion":revision,
                "resolutions":[{"id":"tokenlit","action":"import"}]
            }),
        )
        .await;
    assert_eq!(refused["error"]["data"]["appError"], "mcp_import_conflict");
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("auth.token")
    );
    assert!(!refused.to_string().contains(BEARER_VALUE));

    let user = client.rpc("mcp.catalog", json!({})).await;
    assert_ok(&client.rpc("mcp.save", json!({"source":"user","id":"moved","expectedVersion":user["result"]["userRevision"],"configuration":{"command":"echo","args":["--secret"],"env":{"TOKEN":"fixture-overlay-value"}}})).await);
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&client.rpc("mcp.move", json!({"id":"moved","from":"user","to":"project","cwd":project,"expectedFromVersion":catalog["result"]["userRevision"],"expectedToVersion":catalog["result"]["projectRevision"]})).await);
    let moved = servers_document(json!({
        "moved": {"command":"echo","args":["--new"]}
    }));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":moved}),
        )
        .await;
    assert_ok(&preview);
    assert_eq!(entry(&preview, "moved")["status"], "conflict");
    assert_eq!(fields(entry(&preview, "moved")), ["args"]);
    assert_eq!(entry(&preview, "moved")["confirm"], json!(["args"]));
    let revision = project_revision(&mut client, &project).await;
    let replaced = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":moved,
                "expectedVersion":revision,
                "resolutions":[{"id":"moved","action":"replace","confirm":["args"]}]
            }),
        )
        .await;
    assert_eq!(
        replaced["error"]["data"]["appError"],
        "mcp_credentials_are_user_scoped"
    );
    assert_eq!(project_revision(&mut client, &project).await, revision);
    let renamed = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":moved,
                "expectedVersion":revision,
                "resolutions":[{"id":"moved","action":"rename","newId":"moved-copy","confirm":["args"]}]
            }),
        )
        .await;
    assert_ok(&renamed);
    let withheld = client
        .rpc(
            "mcp.configuration",
            json!({"source":"project","id":"moved","cwd":project}),
        )
        .await;
    assert_ok(&withheld);
    assert_eq!(withheld["result"]["secretsInUserScope"], true);
    assert_eq!(withheld["result"]["configuration"]["argsCount"], 1);
    let copy = client
        .rpc(
            "mcp.configuration",
            json!({"source":"project","id":"moved-copy","cwd":project}),
        )
        .await;
    assert_ok(&copy);
    assert_eq!(copy["result"]["secretsInUserScope"], false);
    assert_eq!(copy["result"]["configuration"]["argsCount"], 1);
    let stored: Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(stored["mcpServers"]["broken"], json!({"command":5}));
    assert_eq!(stored["mcpServers"]["moved-copy"]["args"], json!(["--new"]));
    assert!(stored["mcpServers"]["moved"].get("args").is_none());
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_import_refuses_a_destination_that_keeps_a_withheld_record() {
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
        "plain": {"command":"echo"},
        "free": {"command":"echo"}
    }));
    let preview = client
        .rpc(
            "mcp.import.preview",
            json!({"source":"project","cwd":project,"document":document}),
        )
        .await;
    assert_ok(&preview);
    for id in ["kept", "plain", "free"] {
        assert_eq!(entry(&preview, id)["status"], "changed", "{id}");
    }
    let revision = project_revision(&mut client, &project).await;
    let imported = client
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
        .await;
    assert_eq!(
        imported["error"]["data"]["appError"],
        "mcp_stale_secret_record"
    );
    assert_eq!(
        project_revision(&mut client, &project).await,
        revision,
        "a refused import writes nothing"
    );
    let renamed = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":document,
                "expectedVersion":revision,
                "resolutions":[{"id":"free","action":"rename","newId":"kept"}]
            }),
        )
        .await;
    assert_eq!(
        renamed["error"]["data"]["appError"],
        "mcp_stale_secret_record"
    );
    assert_eq!(project_revision(&mut client, &project).await, revision);
    // A free id with no record imports, and the stale enablement record of the
    // id it writes is withdrawn, so the entry starts disabled (D24).
    let applied = client
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
        .await;
    assert_ok(&applied);
    let catalog = client.rpc("mcp.catalog", json!({"cwd":project})).await;
    assert_ok(&catalog);
    assert_eq!(
        catalog_entry(&catalog, "plain")["enabled"],
        false,
        "an import never enables anything"
    );
    let applied = client
        .rpc(
            "mcp.import.apply",
            json!({
                "source":"project",
                "cwd":project,
                "document":document,
                "expectedVersion":applied["result"]["revision"],
                "resolutions":[{"id":"free","action":"import"}]
            }),
        )
        .await;
    assert_ok(&applied);
    let stored: Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(stored["mcpServers"]["plain"], json!({"command":"echo"}));
    assert_eq!(stored["mcpServers"]["free"], json!({"command":"echo"}));
    assert!(
        stored["mcpServers"].get("kept").is_none(),
        "a refused import writes nothing: {stored}"
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
