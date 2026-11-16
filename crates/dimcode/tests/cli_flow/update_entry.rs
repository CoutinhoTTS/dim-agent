//! `/update`: the terminal UI names the command that installs this copy, and
//! sends nothing to the model. It installs nothing either: the interface is a
//! client of the installation the updater would replace, so the installer
//! cannot run while it serves (cli-commands.md, Update dialog).
use super::*;

/// The opening of the owner's answer for an installation it does not
/// recognise, which is what a binary running out of a build tree is. A
/// release-stamped build names its own entry point instead, and this flow has
/// no npm installation to offer it.
const REFUSAL: &str = "Cannot update this installation";

#[test]
fn update_names_the_command_and_sends_no_input() {
    if dim_agent_capabilities::build::RELEASE {
        return;
    }
    let home = Home::new();
    let (asked, wait_asked) = std::sync::mpsc::channel();
    let daemon = home.daemon(move |wire| {
        wire.hello(&[]);
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
        // A draft that names no command becomes model input, so every
        // `session.submit` between here and the exit is the defect this flow
        // pins. Nothing else is expected but the detach an exit sends.
        let mut methods = Vec::new();
        while let Some(request) = wire.trailing_request() {
            let value: Value = serde_json::from_str(&request).unwrap();
            let method = value["method"].as_str().unwrap_or_default().to_owned();
            methods.push(method);
            wire.reply(&value, json!({}));
        }
        asked.send(methods).unwrap();
    });
    let mut command = home.command();
    sized(&mut command, 120, 32);
    let mut terminal = Terminal::resume_tui(command);
    terminal.until("test / model");

    // The palette offers it, so the entry is discoverable and not only
    // typable: the draft is still being edited while its row is listed.
    terminal.write(b"/upd");
    terminal.until_plain("/update");
    terminal.raw(b"\x03");
    terminal.until_plain("Ask anything...");

    terminal.write(b"/update\r");
    terminal.until_plain(REFUSAL);
    terminal.write(b"/exit\r");
    terminal.until("Bye!");
    assert!(terminal.wait().success(), "{}", terminal.transcript);
    let methods = wait_asked.recv_timeout(PATIENCE).unwrap();
    daemon.join().unwrap();
    assert!(
        !methods.iter().any(|method| method == "session.submit"),
        "{methods:?} followed /update; the draft must not become model input"
    );
}
