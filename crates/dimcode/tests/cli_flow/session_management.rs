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
        for entry in ["session rename SESSION TITLE", "/rename TITLE|--clear"] {
            assert!(help.contains(entry), "{args:?} lacks {entry}");
        }
    }
    assert!(!home.0.join("rust/daemon.sock").exists());
}
