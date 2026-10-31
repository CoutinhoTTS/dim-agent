//! The composer's context reading for a Session method the hello does not
//! list: the reading is asked for like `/usage` asks, and a failed call
//! leaves the border without one and adds no transcript line.
use super::*;
use std::sync::mpsc;

fn usage(input: f64) -> Value {
    json!({"contextWindow":256000.0,"outputTokens":32000.0,"thresholdTokens":200000.0,
        "estimatedInputTokens":input,"baselineSource":"estimate"})
}

fn failed(wire: &mut Wire, request: &Value) {
    wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":
        {"code":-32601,"message":"Unknown method: session.context.usage",
         "data":{"appError":"unknown_method"}}}));
}

fn event_on(wire: &mut Wire, branch: &str, sequence: u64, name: &str, data: Value) {
    wire.notice(
        "session.event",
        json!({"sessionId":"session","branchId":branch,"event":
            {"id":format!("event-{branch}-{sequence}"),"sequence":sequence,"create_at":1,
             "type":"step","name":name,"data":data}}),
    );
}

/// The hello omits `session.context.usage`, which a Session contributes, and
/// the daemon answers it: the border shows `input / window` once the Session
/// is entered and reads it again when a turn on `main` ends.
#[test]
fn tui_shows_context_usage_the_hello_does_not_list() {
    let home = Home::new();
    let (shown, wait_shown) = mpsc::channel();
    let daemon = home.daemon(move |wire| {
        wire.hello(&[]);
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
                Some("session.context.usage") => {
                    assert_eq!(
                        request["params"],
                        json!({"sessionId":"session","branchId":"main"})
                    );
                    wire.reply(&request, usage(1849.0));
                }
                method => panic!("unexpected request {method:?}"),
            }
        }
        wait_shown.recv_timeout(PATIENCE).unwrap();
        let submit = wire.read("session.submit");
        assert_eq!(submit["params"]["text"], "hello");
        wire.reply(&submit, json!({"event":{}}));
        wire.notice(
            "session.state",
            json!({"sessionId":"session","branchId":"main","running":true}),
        );
        wire.event(
            9,
            "text",
            json!({"turn_id":"turn","request_id":"request","delta":"answered"}),
        );
        wire.end(json!({"turn_id":"turn","status":"completed","reason":"fixture"}));
        wire.notice(
            "session.state",
            json!({"sessionId":"session","branchId":"main","running":false}),
        );
        let refresh = wire.read("session.context.usage");
        assert_eq!(
            refresh["params"],
            json!({"sessionId":"session","branchId":"main"})
        );
        wire.reply(&refresh, usage(64000.0));
        let mut line = String::new();
        while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
            assert!(
                !line.contains("session.context.usage"),
                "extra read: {line}"
            );
            line.clear();
        }
    });
    let mut terminal = Terminal::launch_tui(home.command());
    terminal.until("1.8K / 256K");
    shown.send(()).unwrap();
    terminal.write(b"hello\r");
    terminal.until("answered");
    terminal.until("64K / 256K");
    terminal.write(b"/exit\r");
    terminal.until("Bye!");
    assert!(terminal.wait().success());
    daemon.join().unwrap();
}

/// A daemon that answers `session.context.usage` with an error leaves the
/// border without a reading and prints nothing into the transcript, and the
/// next message still goes through.
#[test]
fn tui_shows_no_reading_and_keeps_working_when_context_usage_fails() {
    let home = Home::new();
    let daemon = home.daemon(move |wire| {
        wire.hello(&[]);
        wire.created();
        let attach = wire.read("session.attach");
        wire.reply(&attach, attach_record(&attach));
        let history = wire.read("session.history");
        wire.reply(
            &history,
            json!({"events":[],"through":8,"nextAfter":8,"done":true}),
        );
        let mut asked = false;
        for _ in 0..2 {
            let request = wire.read_any();
            match request["method"].as_str() {
                Some("account.providers") => wire.reply(&request, json!([])),
                Some("session.context.usage") => {
                    asked = true;
                    failed(wire, &request);
                }
                method => panic!("unexpected request {method:?}"),
            }
        }
        assert!(asked, "the TUI did not ask for the context reading");
        let submit = wire.read("session.submit");
        assert_eq!(submit["params"]["text"], "still here");
        wire.reply(&submit, json!({"event":{}}));
        wire.notice(
            "session.state",
            json!({"sessionId":"session","branchId":"main","running":true}),
        );
        wire.event(
            9,
            "text",
            json!({"turn_id":"turn","request_id":"request","delta":"the session still answers"}),
        );
        wire.end(json!({"turn_id":"turn","status":"completed","reason":"fixture"}));
        wire.notice(
            "session.state",
            json!({"sessionId":"session","branchId":"main","running":false}),
        );
        let refresh = wire.read("session.context.usage");
        failed(wire, &refresh);
        let mut line = String::new();
        while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
            line.clear();
        }
    });
    let mut terminal = Terminal::launch_tui(home.command());
    terminal.until("test / model");
    terminal.write(b"still here\r");
    terminal.until("the session still answers");
    terminal.write(b"/exit\r");
    terminal.until("Bye!");
    assert!(terminal.wait().success());
    daemon.join().unwrap();
    for absent in [
        "K / 256K",
        "near the compaction threshold",
        "This daemon does not track context usage",
        "Context usage is known once a model has answered",
        "Context usage needs the model's capabilities",
    ] {
        assert!(
            !terminal.transcript.contains(absent),
            "{absent}: {}",
            terminal.transcript
        );
    }
}

/// A finished model request on `main` reads the context again while the run
/// continues, once per request; a line's request does not read it; the run's
/// end still reads it; and a replayed history page that carries finished
/// requests spends no read of its own (§4).
#[test]
fn tui_context_reading_follows_each_finished_model_request_on_main() {
    let home = Home::new();
    let (shown, wait_shown) = mpsc::channel();
    let (first_rendered, wait_first_rendered) = mpsc::channel();
    let (ended, wait_ended) = mpsc::channel();
    let daemon = home.daemon(move |wire| {
        wire.hello(&[]);
        wire.created();
        let attach = wire.read("session.attach");
        wire.reply(&attach, attach_record(&attach));
        let history = wire.read("session.history");
        wire.reply(
            &history,
            json!({"events":[
                {"sequence":4,"create_at":1,"type":"step","name":"model-output-end",
                 "data":{"turn_id":"old-turn","request_id":"old-request",
                     "status":"completed","finish_reason":"end_turn"}}
            ],"through":8,"nextAfter":8,"done":true}),
        );
        for _ in 0..2 {
            let request = wire.read_any();
            match request["method"].as_str() {
                Some("account.providers") => wire.reply(&request, json!([])),
                Some("session.context.usage") => {
                    assert_eq!(
                        request["params"],
                        json!({"sessionId":"session","branchId":"main"})
                    );
                    wire.reply(&request, usage(1849.0));
                }
                method => panic!("unexpected request {method:?}"),
            }
        }
        wait_shown.recv_timeout(PATIENCE).unwrap();
        let submit = wire.read_any();
        assert_eq!(
            submit["method"], "session.submit",
            "a replayed history page read the context"
        );
        assert_eq!(submit["params"]["text"], "hello");
        wire.reply(&submit, json!({"event":{}}));
        wire.notice(
            "session.state",
            json!({"sessionId":"session","branchId":"main","running":true}),
        );
        event_on(
            wire,
            "main",
            9,
            "model-output-end",
            json!({"turn_id":"turn","request_id":"request-1",
                "status":"completed","finish_reason":"tool_use"}),
        );
        let first = wire.read("session.context.usage");
        assert_eq!(
            first["params"],
            json!({"sessionId":"session","branchId":"main"})
        );
        wire.reply(&first, usage(40000.0));
        // The next request end follows only once the border drew the first
        // reading: the interface drains both updates into one frame
        // otherwise.
        wait_first_rendered.recv_timeout(PATIENCE).unwrap();
        event_on(
            wire,
            "main",
            12,
            "model-output-end",
            json!({"turn_id":"turn","request_id":"request-2",
                "status":"completed","finish_reason":"tool_use"}),
        );
        let second = wire.read("session.context.usage");
        wire.reply(&second, usage(120000.0));
        // A line's finished request is the line's context, not main's.
        event_on(
            wire,
            "line",
            13,
            "model-output-end",
            json!({"turn_id":"line-turn","request_id":"line-request",
                "status":"completed","finish_reason":"end_turn"}),
        );
        let line_history = wire.read("session.history");
        assert_eq!(line_history["params"]["branchId"], "line");
        wire.reply(
            &line_history,
            json!({"events":[],"through":13,"nextAfter":13,"done":true}),
        );
        wire.quiet(Duration::from_millis(300));
        wire.end(json!({"turn_id":"turn","status":"completed","reason":"fixture"}));
        wire.notice(
            "session.state",
            json!({"sessionId":"session","branchId":"main","running":false}),
        );
        let last = wire.read("session.context.usage");
        wire.reply(&last, usage(121000.0));
        ended.send(()).unwrap();
        let mut line = String::new();
        while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
            line.clear();
        }
    });
    let mut terminal = Terminal::launch_tui(home.command());
    terminal.until("1.8K / 256K");
    shown.send(()).unwrap();
    terminal.write(b"hello\r");
    terminal.until("40K / 256K");
    first_rendered.send(()).unwrap();
    terminal.until("120K / 256K");
    wait_ended.recv_timeout(PATIENCE).unwrap();
    terminal.until("121K / 256K");
    terminal.write(b"/exit\r");
    terminal.until("Bye!");
    assert!(terminal.wait().success());
    daemon.join().unwrap();
}
