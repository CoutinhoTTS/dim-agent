//! Ctrl+C ends the interface even when the terminal stopped consuming its
//! output: the exit never waits on a terminal that is not reading.
use super::*;

/// The reported screen: the Goodbye panel reaches the terminal, and the
/// terminal stops consuming output from then on. Two Ctrl+C presses still
/// end the process with the interrupt status.
#[test]
fn ctrl_c_ends_the_interface_when_the_terminal_stops_reading_after_the_goodbye_panel() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        // No `ui.leaf`: the client runs the interface itself.
        wire.hello(&[]);
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
        let mut line = String::new();
        while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
            line.clear();
        }
    });
    let mut terminal = Terminal::launch_tui(home.command());
    terminal.until("test / model");
    terminal.raw(b"\x03");
    terminal.until("Press Ctrl+C again to exit");
    terminal.raw(b"\x03");
    terminal.until("Bye!");
    let status = terminal.wait_without_reading();
    daemon.join().unwrap();
    assert_eq!(status.code(), Some(130), "{}", terminal.transcript);
}

/// The terminal already stopped consuming output before the presses: the
/// renderer is blocked on a full queue while the keys arrive. The exit ends
/// the process the same way.
#[test]
fn ctrl_c_ends_the_interface_when_the_output_queue_is_already_full() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&[]);
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
        let mut line = String::new();
        while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
            line.clear();
        }
    });
    let mut terminal = Terminal::launch_tui(home.command());
    terminal.until("test / model");
    // Typing repaints the frame; unread, the queue fills and the renderer
    // blocks on it before the presses. The first press clears the draft, so
    // the second one exits.
    for _ in 0..3 {
        terminal.raw(&[b'a'; 40]);
        thread::sleep(Duration::from_millis(100));
    }
    assert!(
        terminal.child.try_wait().unwrap().is_none(),
        "the CLI left while the queue filled"
    );
    let waiting = terminal.unread();
    assert!(waiting > 0, "no output was waiting on the terminal");
    terminal.raw(&[b'a'; 40]);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        terminal.unread(),
        waiting,
        "the output queue was not full: the renderer kept writing"
    );
    terminal.raw(b"\x03");
    thread::sleep(Duration::from_millis(100));
    terminal.raw(b"\x03");
    let status = terminal.wait_without_reading();
    daemon.join().unwrap();
    assert_eq!(status.code(), Some(130), "{}", terminal.transcript);
}
