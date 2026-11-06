//! Startup waits follow a daemon that reports real store-upgrade progress:
//! the spawn wait, the start-lock waiter, the one line on stderr and JSON
//! stdout staying clean (#1213).
use super::*;

/// The store upgrade line the CLI prints once.
const UPGRADING: &str =
    "Upgrading the local store from format v33 to v34; this can take a while on a large store.";

fn note_path(home: &Home) -> PathBuf {
    home.0.join("rust/dimcode.sqlite.startup")
}

/// Writes the script as a `--server` program and returns its path.
fn script(home: &Home, name: &str, body: &str) -> PathBuf {
    let path = home.0.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

/// A note written as the daemon writes it: a private temp file renamed over
/// the target. `pid` is expanded by the shell when it is `$$`.
fn note_write(note: &std::path::Path, pid: &str, counter: usize) -> String {
    format!(
        "printf 'dimcode-startup-progress v1 pid=%s\\nstore-upgrade from=33 to=34 progress=%s\\n' \"{pid}\" {counter} > '{note}.tmp.$$'\nchmod 0600 '{note}.tmp.$$'\nmv '{note}.tmp.$$' '{note}'",
        note = note.display()
    )
}

/// A server that reports progress past the shortened start bound and then
/// serves is waited for: the client connects, and says so once on stderr.
#[test]
fn a_starting_daemon_is_waited_for_while_it_reports_progress() {
    let home = Home::new();
    let note = note_path(&home);
    let mut body = String::new();
    for counter in 1..=14 {
        body.push_str(&note_write(&note, "$$", counter));
        body.push_str("\nsleep 0.3\n");
    }
    body.push_str(&format!(
        "exec '{}' app-server \"$@\"",
        env!("CARGO_BIN_EXE_dimcode")
    ));
    let server = script(&home, "slow-server", &body);
    let output = home
        .command()
        .env("DIM_TEST_DAEMON_START_TIMEOUT_MS", "3000")
        .args(["--server", server.to_str().unwrap(), "models", "fixture"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert_eq!(
        stderr.matches(UPGRADING).count(),
        1,
        "the upgrade line is printed once: {stderr}"
    );
    assert!(!note.exists(), "the connected daemon removed the note");
}

/// A child that reports nothing and never serves fails at the bound, with the
/// log location.
#[test]
fn a_starting_daemon_that_reports_nothing_fails_at_the_bound() {
    let home = Home::new();
    let server = script(&home, "silent-server", "exec sleep 5");
    let output = home
        .command()
        .env("DIM_TEST_DAEMON_START_TIMEOUT_MS", "1500")
        .args(["--server", server.to_str().unwrap(), "models", "fixture"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("did not start in time"), "{stderr}");
    assert!(stderr.contains("daemon.log"), "{stderr}");
    assert!(!stderr.contains(UPGRADING), "{stderr}");
}

/// A progress note for another pid does not extend the wait.
#[test]
fn progress_for_another_pid_does_not_extend_the_wait() {
    let home = Home::new();
    let note = note_path(&home);
    let mut body = String::new();
    for counter in 1..=35 {
        body.push_str(&note_write(&note, "999999", counter));
        body.push_str("\nsleep 0.1\n");
    }
    body.push_str("exec sleep 5");
    let server = script(&home, "foreign-server", &body);
    let output = home
        .command()
        .env("DIM_TEST_DAEMON_START_TIMEOUT_MS", "1500")
        .args(["--server", server.to_str().unwrap(), "models", "fixture"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("did not start in time"), "{stderr}");
    assert!(!stderr.contains(UPGRADING), "{stderr}");
}

/// A note written as the daemon does: a private temp file renamed over the
/// target.
fn write_progress_note(note: &std::path::Path, pid: u32, counter: u64) {
    let temp = note.with_file_name(format!(
        "{}.{counter}.tmp",
        note.file_name().unwrap().to_string_lossy()
    ));
    fs::write(
        &temp,
        format!(
            "dimcode-startup-progress v1 pid={pid}\nstore-upgrade from=33 to=34 progress={counter}\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).unwrap();
    fs::rename(&temp, note).unwrap();
}

/// A client waiting on the start lock follows the holder's progress: the
/// test holds the lock, writes notes past the shortened bound, and the waiter
/// is still there when it is released, starts its own daemon, and connects.
#[test]
fn a_start_lock_waiter_follows_progress_past_its_bound() {
    use std::io::BufRead;
    use std::os::unix::fs::OpenOptionsExt;

    let home = Home::new();
    let note = note_path(&home);
    let bound = Duration::from_millis(1500);
    let margin = Duration::from_millis(500);
    // The client's `open_start_lock` derives `<native home>/daemon.start.lock`
    // (crates/dim-cli/src/client.rs:189) and takes it with `try_lock`
    // (client.rs:334); the test holds the same advisory lock, so it waits.
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    options.mode(0o600);
    let lock = options.open(home.0.join("rust/daemon.start.lock")).unwrap();
    lock.lock().unwrap();
    let mut client = home
        .command()
        .env("DIM_TEST_START_LOCK_TIMEOUT_MS", "1500")
        .args(["models", "fixture"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The waiter prints the upgrade line the first time it reads a note, so
    // the line proves it is inside the wait and following the notes.
    let stderr_text = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&stderr_text);
    let pipe = std::io::BufReader::new(client.stderr.take().unwrap());
    let reader = thread::spawn(move || {
        let mut pipe = pipe;
        let mut line = String::new();
        while pipe.read_line(&mut line).unwrap_or(0) > 0 {
            sink.lock().unwrap().push_str(&line);
            line.clear();
        }
    });
    let started = Instant::now();
    let mut counter = 0;
    let mut reported_at: Option<Instant> = None;
    let mut stopped: Option<ExitStatus> = None;
    loop {
        counter += 1;
        write_progress_note(&note, 424242, counter);
        if let Some(status) = client.try_wait().unwrap() {
            stopped = Some(status);
            break;
        }
        if reported_at.is_none() && stderr_text.lock().unwrap().contains(UPGRADING) {
            reported_at = Some(Instant::now());
        }
        // Keep the lock held a whole bound past the note the waiter reported:
        // an inactive waiter would have given up by now, and one that follows
        // the notes is still waiting when the lock is released.
        if let Some(reported) = reported_at
            && reported.elapsed() >= bound + margin
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the waiter neither reported progress nor failed"
        );
        thread::sleep(Duration::from_millis(200));
    }
    let released = Instant::now();
    lock.unlock().unwrap();
    let output = wait(client);
    reader.join().unwrap();
    let stderr = stderr_text.lock().unwrap().clone();
    assert!(
        stopped.is_none(),
        "the client stopped waiting before the lock was released ({stopped:?}): {stderr}"
    );
    assert!(output.status.success(), "{stderr}");
    assert_eq!(stderr.matches(UPGRADING).count(), 1, "{stderr}");
    let reported = reported_at.expect("the waiter reported the progress it saw");
    assert!(
        released.duration_since(reported) >= bound,
        "the lock stayed held a whole bound after the waiter's report"
    );
    assert!(!note.exists(), "the started daemon removed the note");
}

/// JSON mode keeps the upgrade line off stdout.
#[test]
fn the_upgrade_line_stays_off_json_stdout() {
    let home = Home::new();
    let note = note_path(&home);
    let mut body = String::new();
    for counter in 1..=14 {
        body.push_str(&note_write(&note, "$$", counter));
        body.push_str("\nsleep 0.3\n");
    }
    body.push_str(&format!(
        "exec '{}' app-server \"$@\"",
        env!("CARGO_BIN_EXE_dimcode")
    ));
    let server = script(&home, "slow-server", &body);
    let output = home
        .command()
        .env("DIM_TEST_DAEMON_START_TIMEOUT_MS", "3000")
        .args([
            "--server",
            server.to_str().unwrap(),
            "exec",
            "--json",
            "-p",
            "hi",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("Upgrading the local store"), "{stdout}");
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        serde_json::from_str::<Value>(line).expect("stdout is JSON lines");
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.matches(UPGRADING).count(), 1, "{stderr}");
}
