//! `/mcp` and the MCP area through its adapter, over the public wire.
use super::*;
use std::sync::mpsc;

fn hello(wire: &mut Wire) {
    wire.hello(&[
        "mcp.catalog",
        "mcp.configuration",
        "mcp.inspect",
        "mcp.test",
        "mcp.update",
        "mcp.save",
        "mcp.remove",
        "mcp.set_enabled",
        "mcp.disconnect",
        "mcp.auth.start",
        "mcp.auth.complete",
        "mcp.auth.logout",
    ]);
}

/// The effective catalog: a user entry, a project entry and a plugin entry.
fn catalog(docs_enabled: bool, project_error: Option<&str>) -> Value {
    json!({
        "userRevision": "user:1",
        "projectRevision": "project:1",
        "userApplication": {"appliedRevision": "user:1", "error": null, "cleanupError": null},
        "pluginApplication": {"appliedRevision": null, "error": null, "cleanupError": null},
        "projectApplication": {
            "appliedRevision": "project:1",
            "error": project_error,
            "cleanupError": null,
        },
        "servers": [
            {"id": "docs", "source": "user", "enabled": docs_enabled, "transport": "stdio",
             "auth": {"status": "authorized", "missingScopes": ["repo"]}},
            {"id": "metrics", "source": "project", "enabled": false, "transport": "http",
             "auth": {"status": "required"}},
            {"id": "notes", "source": "plugin", "enabled": true, "transport": "stdio"},
        ],
    })
}

fn docs(enabled: bool) -> Value {
    json!({"revision": "user:1", "enabled": enabled, "configuration": {
        "type": "stdio", "command": "npx", "argsCount": 2, "cwd": null,
        "envNames": ["DOCS_TOKEN"],
    }})
}

fn metrics() -> Value {
    json!({"revision": "project:1", "enabled": false, "configuration": {
        "type": "http", "endpoint": "https://metrics.example.com",
        "headerNames": ["X-Key"], "authConfigured": false, "oauthConfigured": true,
        "oauth": {"clientId": "client-1", "callbackPort": 1456, "scopes": "read",
                  "metadataConfigured": false},
    }})
}

fn notes() -> Value {
    json!({"enabled": true, "configuration": {
        "type": "stdio", "command": "notes", "argsCount": 0, "cwd": "/w", "envNames": [],
    }})
}

/// One entry's configuration read, answered in the order the catalog lists
/// its source.
fn configuration(wire: &mut Wire, id: &str, source: &str, reply: Value) {
    let request = wire.read("mcp.configuration");
    assert_eq!(
        request["params"],
        json!({"cwd": "/workspace", "id": id, "source": source})
    );
    wire.reply(&request, reply);
}

/// The whole view read: the catalog, then one configuration per entry.
fn area(wire: &mut Wire, built: Value, docs: Value) {
    let request = wire.read("mcp.catalog");
    wire.reply(&request, built);
    configuration(wire, "docs", "user", docs);
    configuration(wire, "metrics", "project", metrics());
    configuration(wire, "notes", "plugin", notes());
}

/// A write resolves the entry's source from a catalog read of its own.
fn source(wire: &mut Wire) {
    let request = wire.read("mcp.catalog");
    wire.reply(&request, catalog(true, None));
}

/// The Session handshake every terminal flow performs before its own
/// requests.
fn started(wire: &mut Wire) {
    hello(wire);
    wire.created();
    let attach = wire.read("session.attach");
    wire.reply(&attach, attach_record(&attach));
    let history = wire.read("session.history");
    wire.reply(
        &history,
        json!({"events":[],"through":8,"nextAfter":8,"done":true}),
    );
    for _ in 0..2 {
        let request = wire.read_any();
        match request["method"].as_str() {
            Some("account.providers") => wire.reply(&request, json!([])),
            Some("session.context.usage") => wire
                .send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
                "code":-32601,"message":"Unknown method","data":{"appError":"unknown_method"}}})),
            method => panic!("unexpected request {method:?}"),
        }
    }
}

/// `/mcp` lists the effective catalog, and every kind calls the method its
/// design names: Enter toggles, Ctrl+T tests, Tab inspects, Ctrl+X removes;
/// a plugin entry offers no write.
#[test]
fn tui_mcp_panel_lists_manages_and_tests_entries() {
    let home = Home::new();
    let (shown, wait_shown) = mpsc::channel();
    let daemon = home.daemon(move |wire| {
        started(wire);
        wait_shown.recv_timeout(PATIENCE).unwrap();
        area(wire, catalog(true, None), docs(true));
        // Enter disables the selected user entry at the catalog's revision.
        source(wire);
        let request = wire.read("mcp.set_enabled");
        assert_eq!(
            request["params"],
            json!({"cwd": "/workspace", "id": "docs", "source": "user",
                   "expectedVersion": "user:1", "enabled": false})
        );
        wire.reply(&request, json!({}));
        area(wire, catalog(false, None), docs(false));
        // Ctrl+T tests the stored entry; the answer is the tool detail and
        // the area is not read again.
        source(wire);
        let request = wire.read("mcp.test");
        assert_eq!(
            request["params"],
            json!({"cwd": "/workspace", "id": "docs", "source": "user"})
        );
        wire.reply(
            &request,
            json!({"status": "connected", "latencyMs": 5, "tools": [
                {"name": "search", "description": "Search docs",
                 "inputSchema": {"type": "object"}},
            ]}),
        );
        // Tab inspects the effective entry and its connections.
        source(wire);
        let request = wire.read("mcp.inspect");
        assert_eq!(
            request["params"],
            json!({"cwd": "/workspace", "id": "docs", "source": "user"})
        );
        wire.reply(
            &request,
            json!({
                "enabled": false,
                "configuration": {
                    "type": "stdio", "command": "npx", "argsCount": 2, "cwd": null,
                    "envNames": ["DOCS_TOKEN"],
                },
                "connections": [{"state": {"status": "connected"}, "tools": [
                    {"name": "search", "description": "Search docs",
                     "inputSchema": {"type": "object"}},
                ]}],
            }),
        );
        // Ctrl+X asks, Enter removes at the source revision.
        source(wire);
        let request = wire.read("mcp.remove");
        assert_eq!(
            request["params"],
            json!({"cwd": "/workspace", "id": "docs", "source": "user",
                   "expectedVersion": "user:1"})
        );
        wire.reply(&request, json!({}));
        let request = wire.read("mcp.catalog");
        wire.reply(
            &request,
            json!({
                "userRevision": "user:1", "projectRevision": "project:1",
                "userApplication": {"error": null, "cleanupError": null},
                "pluginApplication": {"error": null, "cleanupError": null},
                "projectApplication": {"error": null, "cleanupError": null},
                "servers": [
                    {"id": "metrics", "source": "project", "enabled": false,
                     "transport": "http", "auth": {"status": "required"}},
                    {"id": "notes", "source": "plugin", "enabled": true,
                     "transport": "stdio"},
                ],
            }),
        );
        configuration(wire, "metrics", "project", metrics());
        configuration(wire, "notes", "plugin", notes());
        // /mcp list keeps the report the bare command had.
        let request = wire.read("mcp.catalog");
        wire.reply(&request, catalog(false, None));
        let state = wire.read("session.mcp");
        wire.reply(
            &state,
            json!({"sessionId": "session", "servers": [], "catalogError": null, "closed": false}),
        );
        // Nothing offered a remove for the plugin entry: no mcp.remove may
        // arrive after it.
        let mut line = String::new();
        while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                assert_ne!(value["method"], "mcp.remove", "the plugin entry wrote");
            }
            line.clear();
        }
    });
    let mut terminal = Terminal::launch_tui(home.command());
    terminal.until("test / model");
    shown.send(()).unwrap();
    terminal.write(b"/mcp\r");
    terminal.until("user · stdio · signed in (missing scopes: repo)");
    terminal.until("Environment names: DOCS_TOKEN");
    terminal.raw(b"\r");
    terminal.until("Disabled \"docs\" (user).");
    terminal.raw(b"\x14");
    terminal.until("Tools of docs");
    terminal.until("search — Search docs");
    terminal.raw(b"\x1b");
    thread::sleep(Duration::from_millis(200));
    terminal.raw(b"\t");
    terminal.until("MCP entry docs");
    terminal.until("Connection 1 · connected");
    terminal.raw(b"\x1b");
    thread::sleep(Duration::from_millis(200));
    terminal.raw(b"\x18");
    terminal.until("docs? Enter confirm");
    terminal.raw(b"\r");
    terminal.until("Removed docs.");
    // The plugin entry is selected next; it shows its state and its owner,
    // and Ctrl+X sends nothing.
    terminal.raw(b"\x1b[B");
    terminal.until("read-only");
    terminal.until("Owned by a plugin; change it through Plugins.");
    terminal.raw(b"\x18");
    thread::sleep(Duration::from_millis(200));
    terminal.raw(b"\x1b");
    thread::sleep(Duration::from_millis(200));
    terminal.write(b"/mcp list\r");
    terminal.until("Columns: ID, source, transport, enablement, authorization.");
    terminal.write(b"/exit\r");
    terminal.until("Bye!");
    assert!(terminal.wait().success());
    daemon.join().unwrap();
}

/// An editor that writes a patch for a seed and a new entry for the create
/// template.
fn editor(home: &Home, name: &str, script: &str) -> PathBuf {
    let path = home.0.join(name);
    fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn edit_command(home: &Home, program: &std::path::Path, fullscreen: bool) -> Command {
    let mut command = home.command();
    command.env_remove("VISUAL").env("EDITOR", program);
    if fullscreen {
        command.arg("--fullscreen");
    }
    command
}

const EDITS_THE_ENTRY: &str = "case \"$(cat \"$1\")\" in\n\
    *'\"id\"'*) printf '%s' '{\"id\":\"example\",\"source\":\"user\",\"configuration\":{\"type\":\"stdio\",\"command\":\"npx\",\"args\":[\"-y\",\"server\"],\"env\":{},\"enabled\":false}}' > \"$1\";;\n\
    *) printf '%s' '{\"command\":\"npx\",\"cwd\":\"/workspace\",\"env\":{\"DOCS_TOKEN\":\"value\"}}' > \"$1\";;\n\
    esac";

/// Ctrl+E opens the patch without a stored value, Ctrl+N the create
/// document, and each writes with the method its kind names, in both
/// terminal modes.
#[test]
fn tui_mcp_panel_edits_and_creates_entries() {
    for fullscreen in [false, true] {
        let home = Home::new();
        let script = editor(&home, "editor.sh", EDITS_THE_ENTRY);
        let (shown, wait_shown) = mpsc::channel();
        let daemon = home.daemon(move |wire| {
            started(wire);
            wait_shown.recv_timeout(PATIENCE).unwrap();
            area(wire, catalog(true, None), docs(true));
            // Ctrl+E writes the patch at the revision the view read.
            source(wire);
            let request = wire.read("mcp.update");
            assert_eq!(
                request["params"],
                json!({"cwd": "/workspace", "id": "docs", "source": "user",
                       "expectedVersion": "user:1",
                       "patch": {"command": "npx", "cwd": "/workspace",
                                 "env": {"DOCS_TOKEN": "value"}}})
            );
            wire.reply(&request, json!({}));
            area(wire, catalog(true, None), docs(true));
            // Ctrl+N writes the entry with the source revision, not a
            // revision of its own.
            source(wire);
            let request = wire.read("mcp.save");
            assert_eq!(
                request["params"],
                json!({"cwd": "/workspace", "id": "example", "source": "user",
                       "expectedVersion": "user:1",
                       "configuration": {"type": "stdio", "command": "npx",
                                         "args": ["-y", "server"], "env": {},
                                         "enabled": false}})
            );
            wire.reply(&request, json!({}));
            area(wire, catalog(true, None), docs(true));
            let mut line = String::new();
            while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
                line.clear();
            }
        });
        let mut terminal = Terminal::launch_tui(edit_command(&home, &script, fullscreen));
        terminal.until("test / model");
        shown.send(()).unwrap();
        terminal.write(b"/mcp\r");
        terminal.until("Executable: npx");
        terminal.raw(b"\x05");
        terminal.until("Saved.");
        terminal.raw(b"\x0e");
        terminal.until("Added example.");
        terminal.raw(b"\x1b");
        thread::sleep(Duration::from_millis(200));
        terminal.write(b"/exit\r");
        terminal.until("Bye!");
        assert!(terminal.wait().success());
        daemon.join().unwrap();
    }
}

/// A stale revision writes nothing, says so, reads the area again, and the
/// next edit opens the stored entry, in both terminal modes.
#[test]
fn tui_mcp_panel_keeps_the_users_text_when_the_revision_is_stale() {
    for fullscreen in [false, true] {
        let home = Home::new();
        let script = editor(&home, "editor.sh", EDITS_THE_ENTRY);
        let (shown, wait_shown) = mpsc::channel();
        let daemon = home.daemon(move |wire| {
            started(wire);
            wait_shown.recv_timeout(PATIENCE).unwrap();
            area(wire, catalog(true, None), docs(true));
            source(wire);
            let request = wire.read("mcp.update");
            assert_eq!(request["params"]["expectedVersion"], "user:1");
            wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
                "code":-32000,
                "message":"The MCP settings changed; read them before updating",
                "data":{"appError":"mcp_settings_conflict"}}}));
            // Nothing was written: the area is read again, and the panel
            // moves to the newer revision.
            let request = wire.read("mcp.catalog");
            wire.reply(&request, catalog(true, None));
            let request = wire.read("mcp.configuration");
            wire.reply(
                &request,
                json!({"revision": "user:2", "enabled": true, "configuration": {
                    "type": "stdio", "command": "npx", "argsCount": 2, "cwd": null,
                    "envNames": ["DOCS_TOKEN"],
                }}),
            );
            configuration(wire, "metrics", "project", metrics());
            configuration(wire, "notes", "plugin", notes());
            // The next edit opens the stored entry at that revision.
            source(wire);
            let request = wire.read("mcp.update");
            assert_eq!(request["params"]["expectedVersion"], "user:2");
            assert_eq!(
                request["params"]["patch"],
                json!({"command": "npx", "cwd": "/workspace",
                       "env": {"DOCS_TOKEN": "value"}})
            );
            wire.reply(&request, json!({}));
            area(wire, catalog(true, None), docs(true));
            let mut line = String::new();
            while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
                line.clear();
            }
        });
        let mut terminal = Terminal::launch_tui(edit_command(&home, &script, fullscreen));
        terminal.until("test / model");
        shown.send(()).unwrap();
        terminal.write(b"/mcp\r");
        terminal.until("Executable: npx");
        terminal.raw(b"\x05");
        terminal.until("The MCP entry changed since it was read; nothing was written.");
        terminal.until("text is kept in");
        terminal.raw(b"\x05");
        terminal.until("Saved.");
        terminal.raw(b"\x1b");
        thread::sleep(Duration::from_millis(200));
        terminal.write(b"/exit\r");
        terminal.until("Bye!");
        assert!(terminal.wait().success());
        daemon.join().unwrap();
    }
}

/// A write the owner refuses keeps the draft's revision, so the next edit
/// opens the user's text and writes it again.
#[test]
fn tui_mcp_panel_reports_a_refused_write() {
    let home = Home::new();
    let script = editor(&home, "editor.sh", EDITS_THE_ENTRY);
    let (shown, wait_shown) = mpsc::channel();
    let daemon = home.daemon(move |wire| {
        started(wire);
        wait_shown.recv_timeout(PATIENCE).unwrap();
        area(wire, catalog(true, None), docs(true));
        source(wire);
        let request = wire.read("mcp.update");
        assert_eq!(request["params"]["expectedVersion"], "user:1");
        wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
            "code":-32000,
            "message":"invalid MCP transport configuration",
            "data":{"appError":"mcp_invalid_config"}}}));
        // The same revision writes the same text again.
        source(wire);
        let request = wire.read("mcp.update");
        assert_eq!(request["params"]["expectedVersion"], "user:1");
        assert_eq!(
            request["params"]["patch"],
            json!({"command": "npx", "cwd": "/workspace",
                   "env": {"DOCS_TOKEN": "value"}})
        );
        wire.reply(&request, json!({}));
        area(wire, catalog(true, None), docs(true));
        let mut line = String::new();
        while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
            line.clear();
        }
    });
    let mut terminal = Terminal::launch_tui(edit_command(&home, &script, false));
    terminal.until("test / model");
    shown.send(()).unwrap();
    terminal.write(b"/mcp\r");
    terminal.until("Executable: npx");
    terminal.raw(b"\x05");
    terminal.until("invalid MCP transport configuration");
    terminal.until("text is kept in");
    terminal.raw(b"\x05");
    terminal.until("Saved.");
    terminal.raw(b"\x1b");
    thread::sleep(Duration::from_millis(200));
    terminal.write(b"/exit\r");
    terminal.until("Bye!");
    assert!(terminal.wait().success());
    daemon.join().unwrap();
}

/// Tool text is third-party: an escape sequence and a line break in a
/// description are drawn sanitized, on one row.
#[test]
fn tui_mcp_panel_draws_tool_text_sanitized() {
    let home = Home::new();
    let (shown, wait_shown) = mpsc::channel();
    let daemon = home.daemon(move |wire| {
        started(wire);
        wait_shown.recv_timeout(PATIENCE).unwrap();
        area(wire, catalog(true, None), docs(true));
        source(wire);
        let request = wire.read("mcp.test");
        wire.reply(
            &request,
            json!({"status": "connected", "latencyMs": 3, "tools": [
                {"name": "search", "description": "Search\u{1b}[2J\ndocs",
                 "inputSchema": {"type": "object"}},
            ]}),
        );
        let mut line = String::new();
        while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
            line.clear();
        }
    });
    let mut terminal = Terminal::launch_tui(home.command());
    terminal.until("test / model");
    shown.send(()).unwrap();
    terminal.write(b"/mcp\r");
    terminal.until("Executable: npx");
    terminal.raw(b"\x14");
    terminal.until("search — Search docs");
    assert!(
        !terminal.transcript.contains("[2J"),
        "the escape sequence reaches the screen: {}",
        terminal.transcript
    );
    terminal.signal(libc::SIGKILL);
    terminal.wait();
    daemon.join().unwrap();
}

/// A failed catalog read on opening `/mcp` is not an applied reload: the
/// panel shows the error where the list would be, and no `Reloaded.`.
#[test]
fn tui_mcp_panel_shows_a_failed_catalog_read() {
    let home = Home::new();
    let (shown, wait_shown) = mpsc::channel();
    let daemon = home.daemon(move |wire| {
        started(wire);
        wait_shown.recv_timeout(PATIENCE).unwrap();
        let request = wire.read("mcp.catalog");
        wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
            "code":-32000,
            "message":"MCP settings are unavailable",
            "data":{"appError":"mcp_unavailable"}}}));
        let mut line = String::new();
        while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
            line.clear();
        }
    });
    let mut terminal = Terminal::launch_tui(home.command());
    terminal.until("test / model");
    shown.send(()).unwrap();
    terminal.write(b"/mcp\r");
    terminal.until("MCP settings are unavailable");
    assert!(
        !terminal.transcript.contains("Reloaded."),
        "a failed read was reported as an applied reload: {}",
        terminal.transcript
    );
    terminal.signal(libc::SIGKILL);
    terminal.wait();
    daemon.join().unwrap();
}
