//! Observe fixture-owned processes from the host, including private PID namespaces.
use std::{path::Path, time::Duration};

pub fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

pub async fn gone(pid: &str) -> bool {
    for _ in 0..100 {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

pub async fn pid_in(file: &Path) -> String {
    for _ in 0..500 {
        if let Ok(text) = std::fs::read_to_string(file)
            && text.ends_with('\n')
        {
            #[cfg(not(target_os = "linux"))]
            return text.trim().to_owned();
            #[cfg(target_os = "linux")]
            if let Some(pid) = host_pid(file, text.trim()) {
                return pid;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no observable process for {}", file.display());
}

#[cfg(target_os = "linux")]
fn host_pid(file: &Path, inner_pid: &str) -> Option<String> {
    let marker = file.as_os_str().as_encoded_bytes();
    let mut processes = Vec::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = entry.file_name().into_string() else {
            continue;
        };
        if !pid.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let path = entry.path();
        let Ok(status) = std::fs::read_to_string(path.join("status")) else {
            continue;
        };
        let Some(ids) = status.lines().find_map(|line| line.strip_prefix("NSpid:")) else {
            continue;
        };
        let mut ids = ids.split_whitespace();
        let depth = ids.clone().count();
        let Some(nested) = ids.next_back() else {
            continue;
        };
        let Ok(namespace) = std::fs::read_link(path.join("ns/pid")) else {
            continue;
        };
        let Ok(command) = std::fs::read(path.join("cmdline")) else {
            continue;
        };
        let owns_file = command.windows(marker.len()).any(|bytes| bytes == marker);
        processes.push((pid, namespace, nested.to_owned(), owns_file, depth));
    }
    let namespace = &processes
        .iter()
        .filter(|(_, _, _, owns, _)| *owns)
        .max_by_key(|(_, _, _, _, depth)| *depth)?
        .1;
    processes
        .iter()
        .find(|(_, current, nested, _, _)| current == namespace && nested == inner_pid)
        .map(|(pid, _, _, _, _)| pid.clone())
}
