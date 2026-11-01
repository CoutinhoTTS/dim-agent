//! A test home owns the daemon it started: [`stop_daemon`] ends that daemon
//! before [`remove_directory`] deletes the home, so a test leaves no process
//! and no state behind, including when it fails.

use std::{
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

/// How long a daemon may take to exit after SIGTERM before SIGKILL, and how
/// long the killed process gets to disappear before the directory goes.
const SHUTDOWN: Duration = Duration::from_secs(30);
const KILLED: Duration = Duration::from_secs(5);

/// Stops every process whose command line names `home`.
pub fn stop_daemon(home: &Path) {
    // The CLI writes `rust/daemon.log` before every spawn, so a home without
    // it never started a daemon through the CLI; `rust/daemon.sock.lock` is
    // written by a server started directly on the home. Most homes have
    // neither, and the check is two stats.
    if !home.join("rust/daemon.log").exists() && !home.join("rust/daemon.sock.lock").exists() {
        return;
    }
    let pattern = home.to_str().expect("a test home is UTF-8");
    let mut pids = pids_naming(pattern);
    if pids.is_empty() {
        return;
    }
    for pid in &pids {
        unsafe { libc::kill(*pid, libc::SIGTERM) };
    }
    wait(&pids, SHUTDOWN);
    pids.retain(|pid| running(*pid));
    for pid in &pids {
        unsafe { libc::kill(*pid, libc::SIGKILL) };
    }
    wait(&pids, KILLED);
}

/// The pids of the processes whose command line contains `pattern`.
pub fn pids_naming(pattern: &str) -> Vec<libc::pid_t> {
    let output = match Command::new("pgrep").args(["-f", pattern]).output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("pgrep failed: {error}");
            return Vec::new();
        }
    };
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|pid| pid.parse().ok())
        .collect()
}

fn running(pid: libc::pid_t) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait(pids: &[libc::pid_t], limit: Duration) {
    let deadline = Instant::now() + limit;
    while pids.iter().any(|pid| running(*pid)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
}

/// Removes `directory`, retrying once after a pause for a file a stopping
/// process still creates; a failure is reported on stderr, never panicked.
pub fn remove_directory(directory: &Path) {
    for attempt in 0..2 {
        match std::fs::remove_dir_all(directory) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) if attempt == 1 => {
                eprintln!("could not remove {}: {error}", directory.display());
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
}
