use super::*;

fn fact(id: &str, sequence: u64, at: u64, kind: &str, name: &str, data: Value) -> Value {
    json!({"id":id,"sequence":sequence,"create_at":at,"type":kind,"name":name,"data":data})
}

fn command(id: &str, sequence: u64, at: u64, label: &str) -> Value {
    fact(
        id,
        sequence,
        at,
        "exec",
        "started",
        json!({
            "turn_id":"turn","request_id":"request","call_id":id,"task_id":id,
            "description":label,"output_path":"/server/private/output",
        }),
    )
}

fn delegation(id: &str, sequence: u64, label: &str) -> Value {
    fact(
        id,
        sequence,
        sequence * 1000,
        "collaboration",
        "request",
        json!({
            "turn_id":"turn","request_id":"same-model-request","call_id":"same-model-call",
            "target":{"kind":"agent","agent_id":"reviewer"},"prompt":label,
        }),
    )
}

fn delivery(id: &str, sequence: u64, request: &str) -> Value {
    fact(
        id,
        sequence,
        sequence * 1000,
        "collaboration",
        "delivered",
        json!({
            "op_event_id":request,"status":"ok","agent_id":"reviewer","session_id":"member",
        }),
    )
}

fn state(wire: &mut Wire, branch: &str) {
    wire.notice(
        "session.state",
        json!({"sessionId":"session","branchId":branch,"running":false}),
    );
}

fn event(wire: &mut Wire, branch: &str, event: Value) {
    wire.notice(
        "session.event",
        json!({"sessionId":"session","branchId":branch,"event":event}),
    );
}

#[test]
fn tui_task_inventory_restores_branch_pages_and_retries_only_on_reopen() {
    let home = Home::new();
    let (ready, readiness) = std::sync::mpsc::channel();
    let (proceed, control) = std::sync::mpsc::channel();
    let daemon = home.daemon(move |wire| {
        wire.hello(&[]);
        let summaries = wire.read("session.summaries");
        wire.reply(&summaries, json!([{"sessionId":"session","cwd":"/workspace","model":{"providerId":"test","modelId":"model"},"updatedAt":1}]));
        let attach = wire.read("session.attach");
        wire.reply(&attach, json!({}));
        let main = wire.read("session.history");
        assert_eq!(main["params"]["branchId"], "main");
        wire.reply(&main, json!({"events":[],"through":8,"nextAfter":8,"done":true}));
        let providers = wire.read("account.providers");
        wire.reply(&providers, json!([]));
        // No creation is present in main's local page: initial state discovers the child.
        state(wire, "child");
        let child = wire.read("session.history");
        assert_eq!(child["params"]["branchId"], "child");
        assert_eq!(child["params"]["after"], 0);
        ready.send(()).unwrap();
        control.recv_timeout(Duration::from_secs(10)).unwrap();
        state(wire, "nested");
        state(wire, "child");
        event(wire, "child", fact("child-end", 5, 9000, "exec", "ended", json!({
            "task_id":"child-command","status":"failed","result":{"content":"real child terminal","structuredContent":{"exitCode":7}},
        })));
        // The child read remains the only outstanding page while discovery continues.
        wire.quiet(Duration::from_millis(150));
        wire.reply(&child, json!({"events":[command("child-command",1,1000,"restored child command")],"through":4,"nextAfter":2,"done":false}));
        for _ in 0..2 {
            let read = wire.read("session.history");
            match read["params"]["branchId"].as_str().unwrap() {
                "nested" => wire.reply(&read, json!({"events":[command("nested-command",1,2000,"restored nested command")],"through":1,"nextAfter":1,"done":true})),
                "child" => {
                    assert_eq!(read["params"]["after"], 2);
                    assert_eq!(read["params"]["through"], 4);
                    wire.send(json!({"jsonrpc":"2.0","id":read["id"],"error":{"code":-32000,"message":"page temporarily unavailable"}}));
                }
                other => panic!("unexpected branch {other}"),
            }
        }
        state(wire, "child");
        state(wire, "nested");
        wire.quiet(Duration::from_millis(200));
        ready.send(()).unwrap();
        let retry = wire.read("session.history");
        assert_eq!(retry["params"], json!({"sessionId":"session","branchId":"child","after":2,"through":4,"limit":256}));
        ready.send(()).unwrap();
        control.recv_timeout(Duration::from_secs(10)).unwrap();
        wire.quiet(Duration::from_millis(150));
        wire.reply(&retry, json!({"events":[],"through":4,"nextAfter":4,"done":true}));
        // Finished details are the actual terminal fact, with no live-output request.
        control.recv_timeout(Duration::from_secs(10)).unwrap();
        wire.quiet(Duration::from_millis(200));
        ready.send(()).unwrap();
        let mut line = String::new();
        assert_eq!(wire.0.read_line(&mut line).unwrap(), 0, "unexpected request after finished detail: {line}");
    });
    let mut launch = home.command();
    launch.args(["--resume", "session"]);
    let (mut terminal, mut child) = Terminal::launch_tui(launch);
    terminal.until("test / model");
    readiness.recv_timeout(Duration::from_secs(10)).unwrap();
    terminal.write(b"/tasks\r");
    terminal.until("Loading branch history");
    terminal.write(b"\x18");
    terminal.until("Branch history is incomplete");
    proceed.send(()).unwrap();
    terminal.until("Branch history failed; reopen to retry");
    readiness.recv_timeout(Duration::from_secs(10)).unwrap();
    terminal.write(b"\x1b[27u/tasks\r");
    readiness.recv_timeout(Duration::from_secs(10)).unwrap();
    // Reopening again cannot duplicate an outstanding retry.
    terminal.write(b"\x1b[27u/tasks\r");
    proceed.send(()).unwrap();
    terminal.until("Failed");
    terminal.write(b"restored child\r");
    terminal.until("real child terminal");
    terminal.write(b"x");
    proceed.send(()).unwrap();
    readiness.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(!terminal.transcript.contains("No output yet"));
    child.kill().unwrap();
    child.wait().unwrap();
    daemon.join().unwrap();
}

#[test]
fn tui_task_stop_all_keeps_its_finite_targets_and_waits_only_for_replies() {
    let home = Home::new();
    let (ready, readiness) = std::sync::mpsc::channel();
    let daemon = home.daemon(move |wire| {
        wire.hello(&[]);
        wire.created();
        let attach = wire.read("session.attach");
        wire.reply(&attach, json!({}));
        let history = wire.read("session.history");
        wire.reply(&history, json!({"events":[
            command("main-command",1,1000,"main running command"),
            delegation("request-a",2,"first member delegation"),delivery("delivered-a",3,"request-a"),
            delegation("request-b",4,"second member delegation"),delivery("delivered-b",5,"request-b"),
            delegation("request-starting",6,"still starting member"),
            command("finished-command",7,7000,"finished retained command"),
            fact("finished-end",8,8000,"exec","ended",json!({"task_id":"finished-command","status":"completed","result":{"content":"already complete"}})),
        ],"through":8,"nextAfter":8,"done":true}));
        let providers = wire.read("account.providers");
        wire.reply(&providers, json!([]));
        state(wire, "child");
        let child = wire.read("session.history");
        wire.reply(&child, json!({"events":[command("child-command",1,10000,"child running command")],"through":1,"nextAfter":1,"done":true}));
        let first = wire.read("session.exec.stop");
        assert_eq!(first["params"], json!({"sessionId":"session","branchId":"main","taskId":"main-command"}));
        event(wire, "main", fact("leader-notice",9,9000,"input","message",json!({"mode":"append","user":{"content":[{"type":"text","text":"leader notification while stop waits"}]}})));
        event(wire, "main", command("late-command",10,11000,"arrived after stop admission"));
        // A leader input is accepted while the first stop reply is withheld.
        let submit = wire.read("session.submit");
        assert_eq!(submit["params"]["sessionId"], "session");
        assert_eq!(submit["params"]["text"], "leader stays usable");
        wire.reply(&submit, json!({"event":{}}));
        wire.send(json!({"jsonrpc":"2.0","id":first["id"],"error":{"code":-32000,"message":"first stop failed","data":{"appError":"service_reloading"}}}));
        let member = wire.read("session.stop");
        assert_eq!(member["params"], json!({"sessionId":"member","branchId":"main","reason":"stopped by the user"}));
        wire.quiet(Duration::from_millis(150));
        wire.reply(&member, json!({}));
        let child = wire.read("session.exec.stop");
        assert_eq!(child["params"], json!({"sessionId":"session","branchId":"child","taskId":"child-command"}));
        wire.reply(&child, json!({}));
        // No fourth request for the duplicate member, starting/finished row or new arrival.
        wire.quiet(Duration::from_millis(200));
        ready.send(()).unwrap();
        let mut line = String::new();
        assert_eq!(wire.0.read_line(&mut line).unwrap(), 0, "unexpected extra stop or destructive request: {line}");
    });
    let (mut terminal, mut child) = Terminal::launch_tui(home.command());
    terminal.until("test / model");
    terminal.write(b"/tasks\r");
    terminal.until("child running command");
    terminal.write(b"\x18\x18");
    terminal.until("Stop-all requests are already in progress");
    terminal.write(b"\x1b[27uleader stays usable\r");
    terminal.until("leader notification while stop waits");
    terminal.until("attempted 3, errors 1, unavailable 1");
    readiness.recv_timeout(Duration::from_secs(10)).unwrap();
    terminal.write(b"/tasks\r");
    terminal.until("arrived after stop admission");
    terminal.until("Running");
    assert!(
        !terminal.transcript.contains("Cancelled"),
        "a stop reply fabricated a terminal fact"
    );
    child.kill().unwrap();
    child.wait().unwrap();
    daemon.join().unwrap();
}

#[test]
fn tui_task_inventory_keeps_fifty_terminal_facts_and_their_original_results() {
    let home = Home::new();
    let daemon = home.daemon(move |wire| {
        wire.hello(&[]);
        wire.created();
        let attach = wire.read("session.attach");
        wire.reply(&attach, json!({}));
        let history = wire.read("session.history");
        let mut events = Vec::new();
        for index in 0..52 {
            let id = format!("row-{index:02}");
            let label = if index == 40 { "equal timestamp command".to_owned() } else { id.clone() };
            events.push(command(&id, index * 2 + 1, 500, &label));
            let result = if index == 3 {
                json!({"content":"original result without an exit code"})
            } else {
                json!({"content":format!("original result {index}"),"structuredContent":{"exitCode":index}})
            };
            events.push(fact(&format!("end-{index}"), index * 2 + 2, 1000 + index, "exec", "ended", json!({"task_id":id,"status":"completed","result":result})));
        }
        events.push(delegation("aaa-request",105,"equal timestamp delivery"));
        events.push(fact("failed-delivery",106,1040,"collaboration","delivered",json!({"op_event_id":"aaa-request","status":"error","error":"agent_not_found"})));
        wire.reply(&history, json!({"events":events,"through":106,"nextAfter":106,"done":true}));
        let providers = wire.read("account.providers");
        wire.reply(&providers, json!([]));
        let mut line = String::new();
        assert_eq!(wire.0.read_line(&mut line).unwrap(), 0, "finished details must not send live reads or stops: {line}");
    });
    let (mut terminal, mut child) = Terminal::launch_tui(home.command());
    terminal.until("test / model");
    terminal.write(b"/tasks\r");
    terminal.until("row-51");
    terminal.write(b"row-02");
    terminal.until("No matching tasks.");
    // 53 terminal facts retain 49 commands plus the failed delivery.
    terminal.write(b"\x1b[27urow-03\r");
    terminal.until("original result without an exit code");
    terminal.write(b"x\x1b[27u\x1b[27uequal timestamp\r");
    terminal.until("agent_not_found");
    assert!(!terminal.transcript.contains(" in background"));
    child.kill().unwrap();
    child.wait().unwrap();
    daemon.join().unwrap();
}
