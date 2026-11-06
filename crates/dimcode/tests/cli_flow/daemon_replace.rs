//! Start decisions over the real daemon: one client at a time starts it.
use super::*;

/// Two clients starting together with no daemon start one and both connect.
#[test]
fn two_starting_clients_with_no_daemon_start_one() {
    let home = Home::new();
    let spawn = |home: &Home| {
        home.command()
            .args(["models", "fixture"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let first = spawn(&home);
    let second = spawn(&home);
    let (first, second) = (wait(first), wait(second));
    for (label, output) in [("first", &first), ("second", &second)] {
        assert!(
            output.status.success(),
            "{label}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let log = fs::read_to_string(home.0.join("rust/daemon.log")).unwrap_or_default();
    assert!(
        !log.contains("another process owns the SQLite database"),
        "two daemons raced for the store: {log}"
    );
}
