//! The lines panel and the Session's default branch: the root and the default
//! branch are marked, `p` continues on a line, and `x` ends neither the
//! default branch nor a line it continues from (design §14).
use super::*;

/// A line's creation, as its parent's Timeline carries it.
/// The requests every flow meets besides the ones it asserts, answered here so
/// each flow reads only the requests it is about.
fn next(wire: &mut Wire) -> Value {
    loop {
        let request = wire.read_any();
        let branch = request["params"]["branchId"].clone();
        let through = request["params"]["through"].clone();
        match request["method"].as_str().unwrap_or_default() {
            "session.methods" => {
                let methods: Vec<_> = [
                    "session.input.pending",
                    "session.exec.pending",
                    "session.turn.get",
                ]
                .into_iter()
                .map(|name| json!({"name":name,"mutating":false}))
                .collect();
                wire.reply(&request, json!({"methods":methods}));
            }
            "session.question.pending" => wire.reply(
                &request,
                json!({"branchId":branch,"through":through,"batches":[],"next":null}),
            ),
            "session.input.pending" => wire.reply(
                &request,
                json!({"branchId":branch,"through":through,"inputs":[],"next":null}),
            ),
            "session.exec.pending" => wire.reply(
                &request,
                json!({"branchId":branch,"through":through,"tasks":[],"next":null}),
            ),
            "session.turn.get" => wire.reply(
                &request,
                json!({"branchId":branch,"through":through,"turn":null,"output":null}),
            ),
            "account.providers" => wire.reply(&request, json!([])),
            "provider.list" => wire.reply(&request, json!({"providers":[]})),
            "session.context.usage" => {
                wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
                "code":-32601,"message":"Unknown method: session.context.usage",
                "data":{"appError":"unknown_method"}}}))
            }
            _ => return request,
        }
    }
}

fn created(branch: &str, sequence: u64) -> Value {
    json!({"id":format!("{branch}-create"),"sequence":sequence,"create_at":1,"type":"branch","name":"create","data":{
        "branch":branch,"at":{"kind":"create"},
        "source":{"kind":"tool","turn_id":"turn","request_id":"request","call_id":format!("{branch}-call")},
        "prompt":format!("{branch} work"),
    }})
}

fn history(through: u64, events: Vec<Value>) -> Value {
    json!({"events":events,"through":through,"nextAfter":through,"done":true})
}

fn directory(default_branch_id: &str, lines: &[(&str, Option<&str>)]) -> Value {
    json!({
        "defaultBranchId": default_branch_id,
        "branches": lines.iter().map(|(branch, parent)| match parent {
            Some(parent) => json!({"branchId":branch,"parentBranchId":parent,"branchPoint":1,"state":"open","head":8,"purpose":format!("{branch} work")}),
            None => json!({"branchId":branch,"state":"open","head":8}),
        }).collect::<Vec<_>>(),
    })
}

fn entry(branch: &str, parent: &str) -> Value {
    json!({"branchId":branch,"parentBranchId":parent,"branchPoint":2,"state":"open","head":8,"purpose":format!("{branch} work"),"cutMessages":0,"end":null})
}

/// The lines panel marks the root and the default branch; `p` continues on the
/// selected line, and the next message with no line chosen reaches it.
#[test]
fn tui_lines_panel_marks_the_default_branch_and_p_moves_it() {
    let home = Home::new();
    let (moved, move_seen) = std::sync::mpsc::channel();
    let daemon = home.daemon(move |wire| {
        wire.hello(&[
            "session.methods",
            "session.branches",
            "session.branch.get",
            "session.branch_set_default",
            "session.branch_end",
        ]);
        wire.2 = true;
        let attach = wire.read("session.attach");
        wire.reply(&attach, attach_record(&attach));
        let branches = next(wire);
        assert_eq!(branches["method"], "session.branches");
        wire.reply(
            &branches,
            directory("main", &[("main", None), ("line-1", Some("main"))]),
        );
        let read = next(wire);
        assert_eq!(read["method"], "session.history");
        assert_eq!(read["params"]["branchId"], "main");
        wire.reply(&read, history(8, vec![created("line-1", 2)]));

        let mut moved_on = false;
        loop {
            let request = next(wire);
            match request["method"].as_str().unwrap() {
                "session.branch_set_default" => {
                    assert_eq!(
                        request["params"]["branchId"],
                        json!("line-1"),
                        "p continues on the selected line"
                    );
                    wire.reply(&request, json!({"defaultBranchId":"line-1"}));
                    moved_on = true;
                    moved.send(()).unwrap();
                }
                "session.branch.get" => {
                    assert_eq!(request["params"]["branchId"], json!("line-1"));
                    wire.reply(&request, entry("line-1", "main"));
                }
                "session.history" => {
                    if moved_on {
                        assert_eq!(
                            request["params"]["branchId"],
                            json!("line-1"),
                            "the shown transcript is the line the move reached"
                        );
                    }
                    wire.reply(&request, history(8, vec![]));
                }
                "session.submit" => {
                    assert_eq!(
                        request["params"]["branchId"],
                        json!("line-1"),
                        "a message with no line chosen reaches the default branch"
                    );
                    break;
                }
                other => panic!("unexpected {other}: {request}"),
            }
        }
        wire.detach();
    });
    let mut command = home.command();
    sized(&mut command, 100, 32);
    let mut terminal = Terminal::resume_tui(command);
    terminal.until("test / model");

    terminal.write(b"/lines\r");
    terminal.until("the session itself");
    terminal.until("Main line");
    terminal.until("Start");
    terminal.write(b"\x1b[B");
    terminal.write(b"p");
    move_seen.recv_timeout(PATIENCE).unwrap();
    terminal.write(b"\x1b");
    terminal.until("⑂ main ▸ line-1 work");
    terminal.write(b"hello\r");
    home.kill(&terminal);
    terminal.wait();
    daemon.join().unwrap();
}

/// `x` ends neither the default branch nor a line it continues from, so the
/// end of a line above the role is refused with its own message.
#[test]
fn tui_x_is_refused_on_the_default_branch_and_on_a_line_it_continues_from() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&[
            "session.methods",
            "session.branches",
            "session.branch.get",
            "session.branch_set_default",
            "session.branch_end",
        ]);
        wire.2 = true;
        let attach = wire.read("session.attach");
        wire.reply(&attach, attach_record(&attach));
        let branches = next(wire);
        assert_eq!(branches["method"], "session.branches");
        wire.reply(
            &branches,
            directory(
                "line-2",
                &[
                    ("main", None),
                    ("line-1", Some("main")),
                    ("line-2", Some("line-1")),
                ],
            ),
        );
        loop {
            let request = next(wire);
            match request["method"].as_str().unwrap() {
                "session.history" => {
                    wire.reply(&request, history(8, vec![]));
                }
                "session.branch.get" => {
                    assert_eq!(request["params"]["branchId"], json!("line-2"));
                    wire.reply(&request, entry("line-2", "line-1"));
                }
                other => panic!("unexpected {other}: {request}"),
            }
        }
    });
    let mut command = home.command();
    sized(&mut command, 120, 36);
    let mut terminal = Terminal::resume_tui(command);
    terminal.until("test / model");

    // The panel opens on the shown line, which is the default branch.
    terminal.write(b"/lines\r");
    terminal.until("Main line");
    terminal.write(b"x");
    terminal.until("the default branch and the lines it continues from stay open; continue on another line first");

    // Reopened, the selection is the shown line again; one row up is the
    // line it was opened from.
    terminal.write(b"/lines\r");
    terminal.write(b"\x1b[A");
    terminal.write(b"x");
    terminal.until("the default branch and the lines it continues from stay open; continue on another line first");
    home.kill(&terminal);
    terminal.wait();
    daemon.join().unwrap();
}

/// A refusal from `session.branch_set_default` is drawn in the panel that
/// raised it, not only reported elsewhere.
#[test]
fn tui_a_refused_continue_is_shown_in_the_lines_panel() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&[
            "session.methods",
            "session.branches",
            "session.branch.get",
            "session.branch_set_default",
            "session.branch_end",
        ]);
        wire.2 = true;
        let attach = wire.read("session.attach");
        wire.reply(&attach, attach_record(&attach));
        let branches = next(wire);
        assert_eq!(branches["method"], "session.branches");
        wire.reply(
            &branches,
            directory("main", &[("main", None), ("line-1", Some("main"))]),
        );
        let refused = loop {
            let request = next(wire);
            match request["method"].as_str().unwrap() {
                "session.history" => wire.reply(&request, history(8, vec![created("line-1", 2)])),
                "session.branch.get" => wire.reply(&request, entry("line-1", "main")),
                "session.branch_set_default" => break request,
                other => panic!("unexpected {other}: {request}"),
            }
        };
        assert_eq!(refused["params"]["branchId"], json!("line-1"));
        wire.send(json!({"jsonrpc":"2.0","id":refused["id"],"error":{
            "code":-32001,
            "message":"the line was created through a client before the upgrade, so it cannot be the default branch",
            "data":{"appError":"branch_not_selectable"}}}));
        wire.detach();
    });
    let mut command = home.command();
    sized(&mut command, 120, 40);
    let mut terminal = Terminal::resume_tui(command);
    terminal.until("test / model");

    terminal.write(b"/lines\r");
    terminal.until("the session itself");
    terminal.write(b"\x1b[B");
    terminal.write(b"p");
    terminal.until("so it cannot be the default branch");
    home.kill(&terminal);
    terminal.wait();
    daemon.join().unwrap();
}

/// The shown line resumes from the cursor its attach reply carries: a line's
/// own sequence, and the root's `sequence` when the conversation is the root
/// (design §14).
#[test]
fn tui_resumes_the_shown_line_from_the_attach_cursor() {
    for (line, cursor, entries) in [
        ("main", 8, &[("main", None)][..]),
        ("line-1", 5, &[("main", None), ("line-1", Some("main"))][..]),
    ] {
        let home = Home::new();
        let daemon = home.daemon(move |wire| {
            wire.hello(&[
                "session.methods",
                "session.branches",
                "session.branch.get",
                "session.branch_set_default",
                "session.branch_end",
                "session.history.display",
                "session.history.detail",
                "session.history.display.tail",
                "session.history.display.transcript",
            ]);
            wire.2 = true;
            let attach = wire.read("session.attach");
            let mut record = attach_record(&attach);
            record["defaultBranch"] = json!({"branchId":line,"sequence":cursor});
            wire.reply(&attach, record);
            let branches = next(wire);
            assert_eq!(branches["method"], "session.branches");
            wire.reply(&branches, directory(line, entries));
            let window = next(wire);
            assert_eq!(window["method"], "session.history.display");
            assert_eq!(window["params"]["branchId"], json!(line));
            assert_eq!(
                window["params"]["through"],
                json!(cursor),
                "the shown line resumes from the attach reply's default cursor"
            );
            wire.reply(
                &window,
                json!({"items":[],"through":cursor,"nextBefore":null,"nextAfter":cursor,"done":true}),
            );
            let parent = entries.iter().find(|(id, _)| *id == line).and_then(|(_, parent)| *parent);
            loop {
                let request = next(wire);
                match request["method"].as_str().unwrap() {
                    "session.branch.get" => {
                        assert_eq!(request["params"]["branchId"], json!(line));
                        let detail = match parent {
                            Some(parent) => entry(line, parent),
                            None => json!({"branchId":line,"state":"open","head":8}),
                        };
                        wire.reply(&request, detail);
                    }
                    "session.history" => wire.reply(&request, history(cursor, vec![])),
                    other => panic!("unexpected {other}: {request}"),
                }
            }
        });
        let mut command = home.command();
        sized(&mut command, 100, 32);
        let mut terminal = Terminal::resume_tui(command);
        terminal.until("test / model");
        home.kill(&terminal);
        terminal.wait();
        daemon.join().unwrap();
    }
}
