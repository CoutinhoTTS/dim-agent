//! `dimcode session rename|delete|cleanup` and `/rename`, over the scripted
//! daemon and, for the server guarantees they rely on, the App Server's own
//! flow tests (`crates/dim-app-server/tests/transport_flow/session_title.rs`).
use super::*;
use std::sync::mpsc;

/// The `session.methods` reply of a Session that does or does not serve
/// `session.rename`.
fn methods(rename: bool) -> Value {
    json!({"methods": if rename { vec![json!({"name":"session.rename","mutating":true})] } else { vec![] }})
}

/// The `session.rename` reply for the title the call carried.
fn renamed(request: &Value) -> Value {
    json!({
        "title": request["params"]["title"],
        "titleSource": if request["params"]["title"].is_null() { Value::Null } else { json!("manual") },
        "revision": 1
    })
}

/// Starts the interface on one Session, as the TUI does without `ui.leaf.2`.
fn startup(wire: &mut Wire, extra: &[&str]) {
    wire.hello(extra);
    wire.created();
    let attach = wire.read("session.attach");
    wire.reply(&attach, attach_record(&attach));
    let history = wire.read("session.history");
    wire.reply(
        &history,
        json!({"events":[],"through":8,"nextAfter":8,"done":true}),
    );
    wire.no_context();
    let providers = wire.read("account.providers");
    wire.reply(&providers, json!([]));
}

/// Answers what the interface asks after its start until it leaves, checking
/// that no input carries `forbidden`; returns the titles `session.rename`
/// carried, in order.
fn serve_until_exit(wire: &mut Wire, forbidden: &str) -> Vec<Value> {
    let mut titles = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        match wire.0.read_line(&mut line) {
            Ok(0) => return titles,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => return titles,
        }
        let request: Value = serde_json::from_str(&line).unwrap();
        match request["method"].as_str().unwrap_or_default() {
            "session.methods" => wire.reply(&request, methods(true)),
            "session.rename" => {
                titles.push(request["params"]["title"].clone());
                wire.reply(&request, renamed(&request));
            }
            "session.submit" => {
                let text = request["params"]["text"].as_str().unwrap_or_default();
                assert!(
                    !text.contains(forbidden),
                    "the command reached the model as {text:?}"
                );
                wire.reply(&request, json!({"event":{}}));
            }
            "session.context.usage" => {
                wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
                    "code":-32601,"message":"Unknown method: session.context.usage",
                    "data":{"appError":"unknown_method"}}}));
            }
            "session.idle" => wire.reply(&request, json!({"sequence":8})),
            "session.detach" => wire.reply(&request, json!({})),
            other => panic!("unexpected request {other} {request}"),
        }
    }
}

#[test]
fn session_rename_sets_the_manual_title() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.methods"]);
        let listed = wire.read("session.methods");
        assert_eq!(listed["params"], json!({"sessionId":"session"}));
        wire.reply(&listed, methods(true));
        let rename = wire.read("session.rename");
        assert_eq!(
            rename["params"],
            json!({"sessionId":"session","title":"Manual title"})
        );
        wire.reply(&rename, renamed(&rename));
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "rename", "session", "Manual title"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Renamed to Manual title\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn session_rename_clear_sends_a_null_title() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.methods"]);
        let listed = wire.read("session.methods");
        wire.reply(&listed, methods(true));
        let rename = wire.read("session.rename");
        assert_eq!(
            rename["params"],
            json!({"sessionId":"session","title":null})
        );
        wire.reply(&rename, renamed(&rename));
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "rename", "session", "--clear"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "The Session is now untitled.\n"
    );
}

#[test]
fn session_rename_without_exactly_one_title_is_a_usage_error() {
    let home = Home::new();
    for args in [
        vec!["session", "rename", "session"],
        vec!["session", "rename", "session", "Manual title", "--clear"],
    ] {
        let output = home.command().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Use session rename SESSION TITLE or session rename SESSION --clear"),
            "{args:?}: {stderr}"
        );
        assert!(
            !home.0.join("rust/daemon.sock").exists(),
            "a usage error connects to no daemon"
        );
    }
}

#[test]
fn session_rename_reports_a_daemon_without_the_method() {
    for listed in [None, Some(false)] {
        let home = Home::new();
        let daemon = home.daemon(move |wire| {
            wire.hello(if listed.is_some() {
                &["session.methods"]
            } else {
                &[]
            });
            if let Some(serve) = listed {
                let request = wire.read("session.methods");
                wire.reply(&request, methods(serve));
            }
            wire.hold();
        });
        let output = home
            .command()
            .args(["session", "rename", "session", "Manual title"])
            .output()
            .unwrap();
        daemon.join().unwrap();
        assert_eq!(output.status.code(), Some(1), "{listed:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("This daemon does not support renaming Sessions"),
            "{listed:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn session_rename_reports_an_unknown_session() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.methods"]);
        let listed = wire.read("session.methods");
        wire.send(json!({"jsonrpc":"2.0","id":listed["id"],"error":{
            "code":-32000,"message":"Session is closed",
            "data":{"appError":"session_closed"}}}));
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "rename", "gone", "Manual title"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Session is closed"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn tui_rename_sets_and_clears_the_title_without_submitting_it() {
    for fullscreen in [false, true] {
        let home = Home::new();
        let daemon = home.daemon(move |wire| {
            startup(wire, &["session.methods"]);
            let titles = serve_until_exit(wire, "/rename");
            assert_eq!(titles, vec![json!("Foo"), Value::Null]);
        });
        let mut command = home.command();
        command.arg(if fullscreen {
            "--fullscreen"
        } else {
            "--scrollback"
        });
        let mut terminal = Terminal::launch_tui(command);
        terminal.until("test / model");
        terminal.write(b"/rename Foo\r");
        terminal.until("Renamed to Foo");
        terminal.write(b"/rename --clear\r");
        terminal.until("The Session is now untitled.");
        terminal.write(b"/exit\r");
        terminal.until("Bye!");
        assert!(terminal.wait().success(), "{}", terminal.transcript);
        daemon.join().unwrap();
    }
}

#[test]
fn tui_palette_rename_waits_for_the_title() {
    for fullscreen in [false, true] {
        for key in [b"\r".as_slice(), b"\t".as_slice()] {
            let home = Home::new();
            let daemon = home.daemon(move |wire| {
                startup(wire, &["session.methods"]);
                let titles = serve_until_exit(wire, "/rename");
                assert_eq!(titles, vec![json!("Foo")], "Enter or Tab renamed early");
            });
            let mut command = home.command();
            command.arg(if fullscreen {
                "--fullscreen"
            } else {
                "--scrollback"
            });
            let mut terminal = Terminal::launch_tui(command);
            terminal.until("test / model");
            terminal.write(b"/ren");
            terminal.raw(key);
            terminal.write(b"Foo\r");
            terminal.until("Renamed to Foo");
            terminal.write(b"/exit\r");
            terminal.until("Bye!");
            assert!(terminal.wait().success(), "{}", terminal.transcript);
            daemon.join().unwrap();
        }
    }
}

#[test]
fn tui_rename_alone_is_refused_with_the_needs_notice() {
    for fullscreen in [false, true] {
        let home = Home::new();
        let daemon = home.daemon(|wire| {
            startup(wire, &["session.methods"]);
            let titles = serve_until_exit(wire, "/rename");
            assert!(titles.is_empty(), "{titles:?}");
        });
        let mut command = home.command();
        command.arg(if fullscreen {
            "--fullscreen"
        } else {
            "--scrollback"
        });
        let mut terminal = Terminal::launch_tui(command);
        terminal.until("test / model");
        terminal.write(b"/rename");
        terminal.raw(b"\r");
        terminal.raw(b"\r");
        terminal.until("/rename needs <title>");
        terminal.raw(b"\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f");
        terminal.write(b"/exit\r");
        terminal.until("Bye!");
        assert!(terminal.wait().success(), "{}", terminal.transcript);
        daemon.join().unwrap();
    }
}

#[test]
fn tui_rename_during_a_running_turn_is_a_command_not_a_queued_input() {
    for fullscreen in [false, true] {
        let home = Home::new();
        let (running, is_running) = mpsc::channel();
        let daemon = home.daemon(move |wire| {
            startup(wire, &["session.methods"]);
            let submit = wire.read("session.submit");
            assert_eq!(submit["params"]["text"], "hello");
            wire.reply(&submit, json!({"event":{}}));
            wire.notice(
                "session.state",
                json!({"sessionId":"session","branchId":"main","running":true,"stopping":false}),
            );
            running.send(()).unwrap();
            let mut line = String::new();
            loop {
                line.clear();
                match wire.0.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        continue;
                    }
                    Err(_) => break,
                }
                let request: Value = serde_json::from_str(&line).unwrap();
                match request["method"].as_str().unwrap_or_default() {
                    "session.methods" => wire.reply(&request, methods(true)),
                    "session.rename" => {
                        assert_eq!(request["params"]["title"], "Foo");
                        wire.reply(&request, renamed(&request));
                    }
                    "session.submit" => {
                        let text = request["params"]["text"].as_str().unwrap_or_default();
                        assert!(
                            !text.contains("/rename"),
                            "the command was queued as an input: {text:?}"
                        );
                        wire.reply(&request, json!({"event":{}}));
                    }
                    "session.context.usage" => wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
                        "code":-32601,"message":"Unknown method","data":{"appError":"unknown_method"}}})),
                    "session.idle" => wire.reply(&request, json!({"sequence":8})),
                    "session.detach" => wire.reply(&request, json!({})),
                    other => panic!("unexpected request {other} {request}"),
                }
            }
        });
        let mut command = home.command();
        command.arg(if fullscreen {
            "--fullscreen"
        } else {
            "--scrollback"
        });
        let mut terminal = Terminal::launch_tui(command);
        terminal.until("test / model");
        terminal.write(b"hello\r");
        is_running.recv_timeout(PATIENCE).unwrap();
        terminal.until("esc interrupt");
        terminal.write(b"/rename Foo\r");
        terminal.until("Renamed to Foo");
        terminal.write(b"/exit\r");
        terminal.until("Bye!");
        assert!(terminal.wait().success(), "{}", terminal.transcript);
        daemon.join().unwrap();
    }
}

#[test]
fn tui_rename_without_a_session_sends_nothing() {
    for fullscreen in [false, true] {
        let home = Home::new();
        let daemon = home.daemon(|wire| {
            startup(wire, &["session.methods"]);
            let titles = serve_until_exit(wire, "/rename");
            assert!(titles.is_empty(), "{titles:?}");
        });
        let mut command = home.command();
        command.arg(if fullscreen {
            "--fullscreen"
        } else {
            "--scrollback"
        });
        let mut terminal = Terminal::launch_tui(command);
        terminal.until("test / model");
        terminal.write(b"/new\r");
        terminal.until("new session");
        terminal.write(b"/rename Foo\r");
        terminal.until("/rename needs a session");
        terminal.write(b"/exit\r");
        terminal.until("Bye!");
        assert!(terminal.wait().success(), "{}", terminal.transcript);
        daemon.join().unwrap();
    }
}

#[test]
fn tui_rename_reports_a_daemon_without_the_method() {
    for fullscreen in [false, true] {
        let home = Home::new();
        let daemon = home.daemon(|wire| {
            startup(wire, &[]);
            let mut line = String::new();
            loop {
                line.clear();
                match wire.0.read_line(&mut line) {
                    Ok(0) => return,
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        continue;
                    }
                    Err(_) => return,
                }
                let request: Value = serde_json::from_str(&line).unwrap();
                match request["method"].as_str().unwrap_or_default() {
                    "session.submit" => {
                        let text = request["params"]["text"].as_str().unwrap_or_default();
                        assert!(!text.contains("/rename"), "{text:?}");
                        wire.reply(&request, json!({"event":{}}));
                    }
                    "session.context.usage" => wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
                        "code":-32601,"message":"Unknown method","data":{"appError":"unknown_method"}}})),
                    "session.idle" => wire.reply(&request, json!({"sequence":8})),
                    "session.detach" => wire.reply(&request, json!({})),
                    other => panic!("unexpected request {other} {request}"),
                }
            }
        });
        let mut command = home.command();
        command.arg(if fullscreen {
            "--fullscreen"
        } else {
            "--scrollback"
        });
        let mut terminal = Terminal::launch_tui(command);
        terminal.until("test / model");
        terminal.write(b"/rename Foo\r");
        terminal.until("This daemon does not support renaming Sessions");
        terminal.write(b"/exit\r");
        terminal.until("Bye!");
        assert!(terminal.wait().success(), "{}", terminal.transcript);
        daemon.join().unwrap();
    }
}

#[test]
fn help_lists_the_session_commands() {
    let home = Home::new();
    for args in [vec!["--help"], vec!["session", "--help"]] {
        let output = home.command().args(&args).output().unwrap();
        assert!(output.status.success(), "{args:?}");
        let help = String::from_utf8(output.stdout).unwrap();
        for entry in [
            "session rename SESSION TITLE",
            "session delete SESSION",
            "session cleanup",
            "/rename TITLE|--clear",
        ] {
            assert!(help.contains(entry), "{args:?} lacks {entry}");
        }
    }
    assert!(!home.0.join("rust/daemon.sock").exists());
}

/// A stored Session as `session.summaries` reports it, `ago` milliseconds
/// before now.
fn summary(id: &str, title: Option<&str>, ago: u64) -> Value {
    json!({
        "sessionId": id,
        "cwd": "/workspace",
        "model": {"providerId":"test","modelId":"model"},
        "createdAt": 1,
        "running": false,
        "sequence": 8,
        "attachedClients": 0,
        "updatedAt": now_ms().saturating_sub(ago),
        "title": title,
        "closed": false,
    })
}

/// `summary` with the closed and activity fields the cleanup preview reads.
fn state(mut session: Value, closed: bool, running: bool, clients: usize) -> Value {
    session["closed"] = json!(closed);
    session["running"] = json!(running);
    session["attachedClients"] = json!(clients);
    session
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

const HOUR: u64 = 60 * 60 * 1000;

/// Three stored Sessions, the newest first as the panel shows them: `c` was
/// active an hour ago, `b` a day ago and `a` three days ago.
fn three_sessions() -> Value {
    json!([
        summary("c", Some("Newest"), HOUR),
        summary("b", Some("Middle"), 24 * HOUR + HOUR),
        summary("a", None, 3 * 24 * HOUR + HOUR),
    ])
}

#[test]
fn session_delete_reports_a_deleted_session() {
    // The Session the daemon deletes is not in its summaries, as a Session
    // it closed is not: the command must delete without reading them.
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.delete", "session.summaries"]);
        let delete = wire.read("session.delete");
        assert_eq!(delete["params"], json!({"sessionId":"session"}));
        wire.reply(&delete, json!({"deleted": true}));
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "delete", "session"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Deleted session\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn session_delete_reports_a_delete_that_removed_nothing() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.delete"]);
        let delete = wire.read("session.delete");
        assert_eq!(delete["params"], json!({"sessionId":"gone"}));
        wire.reply(&delete, json!({"deleted": false}));
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "delete", "gone"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .contains("Nothing was deleted: no Session gone is stored in this daemon's database."),
        "{stderr}"
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn session_delete_reads_a_reply_without_the_field_as_a_delete() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.delete"]);
        let delete = wire.read("session.delete");
        wire.reply(&delete, json!({}));
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "delete", "session"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Deleted session\n"
    );
}

#[test]
fn session_delete_reports_a_daemon_without_the_method() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.summaries"]);
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "delete", "session"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("This daemon does not support deleting Sessions"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn session_cleanup_selects_by_age_and_keeps_nothing_without_a_terminal() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.summaries", "session.delete"]);
        let listed = wire.read("session.summaries");
        wire.reply(&listed, three_sessions());
        // No `session.delete`: the run refuses before deleting anything.
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "cleanup", "--older-than", "2d"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("pass --yes"), "{stderr}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        ["a\t-\t3d ago"],
        "the selection is the Session older than two days"
    );
}

#[test]
fn session_cleanup_by_keep_deletes_exactly_the_selection() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.summaries", "session.delete"]);
        let listed = wire.read("session.summaries");
        wire.reply(&listed, three_sessions());
        for expected in ["b", "a"] {
            let delete = wire.read("session.delete");
            assert_eq!(delete["params"], json!({"sessionId": expected}));
            wire.reply(&delete, json!({}));
        }
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "cleanup", "--keep", "1", "--yes"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [
            "b\tMiddle\t1d ago",
            "a\t-\t3d ago",
            "Deleted b",
            "Deleted a",
        ],
        "the preview names the selection, then each delete reports"
    );
}

#[test]
fn session_cleanup_names_a_failed_id_and_prints_its_retry() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.summaries", "session.delete"]);
        let listed = wire.read("session.summaries");
        wire.reply(&listed, three_sessions());
        let first = wire.read("session.delete");
        assert_eq!(first["params"], json!({"sessionId":"b"}));
        wire.send(json!({"jsonrpc":"2.0","id":first["id"],"error":{
            "code":-32000,"message":"fixture storage_failed",
            "data":{"appError":"storage_failed"}}}));
        let second = wire.read("session.delete");
        assert_eq!(second["params"], json!({"sessionId":"a"}));
        wire.reply(&second, json!({}));
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "cleanup", "--keep", "1", "--yes"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cannot delete b: fixture storage_failed"),
        "{stderr}"
    );
    assert!(
        stderr.contains("dimcode session delete b"),
        "the failed id has a retry command: {stderr}"
    );
    assert!(!stderr.contains("dimcode session delete a"), "{stderr}");
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Deleted a"),
        "the other selected Session is still deleted"
    );
}

#[test]
fn session_cleanup_with_an_empty_selection_deletes_nothing() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.summaries", "session.delete"]);
        let listed = wire.read("session.summaries");
        wire.reply(&listed, three_sessions());
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "cleanup", "--keep", "9", "--yes"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "No Sessions matched; nothing was deleted.\n"
    );
}

#[test]
fn session_cleanup_asks_on_a_terminal() {
    for answer in ["n", "y"] {
        let home = Home::new();
        let daemon = home.daemon(move |wire| {
            wire.hello(&["session.summaries", "session.delete"]);
            let listed = wire.read("session.summaries");
            wire.reply(&listed, three_sessions());
            if answer == "y" {
                for expected in ["b", "a"] {
                    let delete = wire.read("session.delete");
                    assert_eq!(delete["params"], json!({"sessionId": expected}));
                    wire.reply(&delete, json!({}));
                }
            }
            wire.hold();
        });
        let mut command = home.command();
        command.args(["session", "cleanup", "--keep", "1"]);
        let mut terminal = Terminal::launch_tui(command);
        terminal.until("Delete 2 Sessions? [y/N]");
        terminal.write(format!("{answer}\n").as_bytes());
        terminal.until(if answer == "y" {
            "Deleted a"
        } else {
            "Nothing was deleted."
        });
        assert!(terminal.wait().success(), "{}", terminal.transcript);
        daemon.join().unwrap();
    }
}

#[test]
fn session_cleanup_reports_a_daemon_without_the_method() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.summaries"]);
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "cleanup", "--keep", "1", "--yes"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("This daemon does not support cleaning up Sessions"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn session_cleanup_requires_exactly_one_selector() {
    let home = Home::new();
    for (args, expected) in [
        (
            &["session", "cleanup"][..],
            "Use session cleanup --older-than DURATION or session cleanup --keep N",
        ),
        (
            &["session", "cleanup", "--keep", "1", "--older-than", "3d"][..],
            "Use session cleanup --older-than DURATION or session cleanup --keep N",
        ),
        (
            &["session", "cleanup", "--older-than", "3"][..],
            "--older-than takes a number and a unit d, h or m",
        ),
    ] {
        let output = home.command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{args:?}: {stderr}");
    }
}

#[test]
fn session_cleanup_rejects_a_duration_the_units_do_not_cover() {
    let home = Home::new();
    for value in ["30日", "d", "30", "", "1w", "3 days"] {
        let output = home
            .command()
            .args(["session", "cleanup", "--older-than", value, "--yes"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "{value:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("--older-than takes a number and a unit d, h or m"),
            "{value:?}: {stderr}"
        );
        assert!(output.stdout.is_empty(), "{value:?}");
    }
    assert!(
        !home.0.join("rust/daemon.sock").exists(),
        "a usage error connects to no daemon"
    );
}

#[test]
fn session_cleanup_lists_and_deletes_a_closed_session() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.summaries", "session.delete"]);
        let listed = wire.read("session.summaries");
        assert_eq!(listed["params"], json!({"includeClosed": true}));
        wire.reply(
            &listed,
            json!([
                summary("kept", Some("Newest"), HOUR),
                state(summary("closed", None, 3 * HOUR), true, false, 0),
            ]),
        );
        let delete = wire.read("session.delete");
        assert_eq!(delete["params"], json!({"sessionId":"closed"}));
        wire.reply(&delete, json!({"deleted": true}));
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "cleanup", "--keep", "1", "--yes"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["closed\t-\t3h ago\tclosed", "Deleted closed"],
        "a closed Session is shown as closed and deleted"
    );
}

#[test]
fn session_cleanup_keeps_active_sessions() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.summaries", "session.delete"]);
        let listed = wire.read("session.summaries");
        wire.reply(
            &listed,
            json!([
                state(
                    summary("running", Some("Running"), 3 * 24 * HOUR),
                    false,
                    true,
                    0
                ),
                state(
                    summary("attached", Some("Attached"), 4 * 24 * HOUR),
                    false,
                    false,
                    2
                ),
                summary("idle", Some("Idle"), 5 * 24 * HOUR),
            ]),
        );
        let delete = wire.read("session.delete");
        assert_eq!(
            delete["params"],
            json!({"sessionId":"idle"}),
            "an active Session is never deleted"
        );
        wire.reply(&delete, json!({"deleted": true}));
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "cleanup", "--older-than", "2d", "--yes"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "idle\tIdle\t5d ago",
            "Kept (active):",
            "running\tRunning\t3d ago\trunning",
            "attached\tAttached\t4d ago\t2 clients attached",
            "Deleted idle",
        ]
    );
}

#[test]
fn session_cleanup_with_keep_zero_selects_every_inactive_session() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.summaries", "session.delete"]);
        let listed = wire.read("session.summaries");
        wire.reply(
            &listed,
            json!([
                state(summary("c", Some("Newest"), HOUR), false, true, 1),
                summary("b", Some("Middle"), 24 * HOUR + HOUR),
                summary("a", None, 3 * 24 * HOUR + HOUR),
            ]),
        );
        for expected in ["b", "a"] {
            let delete = wire.read("session.delete");
            assert_eq!(delete["params"], json!({"sessionId": expected}));
            wire.reply(&delete, json!({"deleted": true}));
        }
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "cleanup", "--keep", "0", "--yes"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "b\tMiddle\t1d ago",
            "a\t-\t3d ago",
            "Kept (active):",
            "c\tNewest\t1h ago\trunning, 1 client attached",
            "Deleted b",
            "Deleted a",
        ]
    );
}

#[test]
fn session_cleanup_reports_every_failed_delete() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.summaries", "session.delete"]);
        let listed = wire.read("session.summaries");
        wire.reply(
            &listed,
            json!([
                state(summary("c", Some("Newest"), HOUR), false, true, 1),
                summary("b", Some("Middle"), 24 * HOUR + HOUR),
                summary("a", None, 3 * 24 * HOUR + HOUR),
            ]),
        );
        for expected in ["b", "a"] {
            let delete = wire.read("session.delete");
            assert_eq!(delete["params"], json!({"sessionId": expected}));
            wire.send(json!({"jsonrpc":"2.0","id":delete["id"],"error":{
                "code":-32000,"message":"fixture storage_failed",
                "data":{"appError":"storage_failed"}}}));
        }
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "cleanup", "--keep", "0", "--yes"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    for id in ["b", "a"] {
        assert!(
            stderr.contains(&format!("Cannot delete {id}: fixture storage_failed")),
            "{stderr}"
        );
        assert!(
            stderr.contains(&format!("dimcode session delete {id}")),
            "the failed id has a retry command: {stderr}"
        );
    }
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("Deleted"),
        "no delete succeeded"
    );
}

#[test]
fn session_cleanup_falls_back_once_without_include_closed() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&["session.summaries", "session.delete"]);
        let listed = wire.read("session.summaries");
        assert_eq!(listed["params"], json!({"includeClosed": true}));
        wire.send(json!({"jsonrpc":"2.0","id":listed["id"],"error":{
            "code":-32602,"message":"Invalid request parameters",
            "data":{"appError":"invalid_params"}}}));
        let retried = wire.read("session.summaries");
        assert_eq!(retried["params"], json!({}), "the field is dropped once");
        wire.reply(&retried, json!([]));
        wire.hold();
    });
    let output = home
        .command()
        .args(["session", "cleanup", "--keep", "1", "--yes"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(
            "This App Server does not list Sessions closed since it started; they are not included."
        ),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "No Sessions matched; nothing was deleted.\n"
    );
}
