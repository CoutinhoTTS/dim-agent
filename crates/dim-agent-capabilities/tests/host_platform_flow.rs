//! HostPlatform acceptance on Unix (`docs/architecture/rust-agent/plugins/HostPlatform.md`).
#![cfg(unix)]
use dim_agent_capabilities::host_platform::*;
use std::{ffi::OsString, path::PathBuf, time::Duration};
use tokio::io::AsyncReadExt;

fn env(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs
        .iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect()
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes the process.
    unsafe { libc::kill(pid, 0) == 0 }
}

async fn gone(pid: i32) -> bool {
    // An orphaned grandchild is reaped by init shortly after it exits.
    for _ in 0..100 {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn shell_resolution_follows_the_preference_and_never_fails_the_service() {
    let path = env(&[("PATH", "/usr/bin:/bin"), ("SHELL", "/bin/sh")]);
    let ShellStatus::Available(login) = resolve_shell(Ok(None), &path).await else {
        panic!("a POSIX host has /bin/sh")
    };
    assert_eq!(login.executable_path, PathBuf::from("/bin/sh"));
    assert_eq!(login.edition, ShellEdition::Sh);
    assert_eq!(login.source, ShellSource::Login);
    assert_eq!(login.output_encoding, OutputEncoding::Utf8);

    let explicit = br#"{"version":1,"executablePath":"/bin/sh"}"#.to_vec();
    let ShellStatus::Available(chosen) = resolve_shell(Ok(Some(explicit)), &path).await else {
        panic!("/bin/sh is a candidate")
    };
    assert_eq!(chosen.source, ShellSource::Explicit);

    let missing = br#"{"version":1,"executablePath":"/nonexistent/zsh"}"#.to_vec();
    match resolve_shell(Ok(Some(missing)), &path).await {
        ShellStatus::Unavailable { code, reason } => {
            assert_eq!(code, EXEC_SHELL_UNAVAILABLE);
            assert!(reason.contains("/nonexistent/zsh"), "{reason}");
        }
        other => panic!("an explicit path never falls back: {other:?}"),
    }
    for invalid in [
        &br#"{"version":2}"#[..],
        br#"{"version":1,"shell":"bash"}"#,
        br#"{"version":1,"executablePath":"bin/bash"}"#,
        br#"{"version":1,"executablePath":""}"#,
        br#"{"executablePath":"/bin/sh"}"#,
        b"not json",
    ] {
        assert!(matches!(
            resolve_shell(Ok(Some(invalid.to_vec())), &path).await,
            ShellStatus::Unavailable {
                code: EXEC_SHELL_CONFIG_INVALID,
                ..
            }
        ));
    }
    assert!(matches!(
        resolve_shell(Err("unreadable".into()), &path).await,
        ShellStatus::Unavailable {
            code: EXEC_SHELL_CONFIG_INVALID,
            ..
        }
    ));
}

#[tokio::test]
async fn spawn_isolates_the_environment_and_terminate_releases_the_tree() {
    let platform = HostPlatform::current(ShellStatus::Unavailable {
        code: EXEC_SHELL_UNAVAILABLE,
        reason: "spawn does not need a shell".into(),
    })
    .unwrap();
    let cwd = std::env::temp_dir();

    // Only the given environment; stdin is empty.
    let mut tree = platform
        .spawn(ProcessSpec {
            executable: "/bin/sh".into(),
            args: vec!["-c".into(), "env; cat".into()],
            cwd: cwd.clone(),
            env: env(&[("ONLY", "this")]),
        })
        .unwrap();
    let mut output = String::new();
    tree.stdout().read_to_string(&mut output).await.unwrap();
    assert!(tree.wait().await.unwrap().success());
    assert!(output.contains("ONLY=this"), "{output}");
    assert!(!output.contains("HOME="), "{output}");

    // A grandchild dies with the tree.
    let spawn_tree = || {
        platform
            .spawn(ProcessSpec {
                executable: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 100 & echo $!; wait".into()],
                cwd: cwd.clone(),
                env: vec![],
            })
            .unwrap()
    };
    let grandchild = |tree: &mut ProcessTree| {
        let mut stdout = tree.stdout();
        async move {
            let mut line = Vec::new();
            let mut byte = [0u8];
            while stdout.read(&mut byte).await.unwrap() == 1 && byte[0] != b'\n' {
                line.push(byte[0]);
            }
            String::from_utf8(line).unwrap().parse::<i32>().unwrap()
        }
    };
    let mut tree = spawn_tree();
    let pid = grandchild(&mut tree).await;
    assert!(alive(pid));
    tree.terminate().await.unwrap();
    assert!(gone(pid).await, "terminate left the grandchild");

    // Dropping an unterminated handle kills the tree as the last resort.
    let mut tree = spawn_tree();
    let pid = grandchild(&mut tree).await;
    drop(tree);
    assert!(gone(pid).await, "drop left the grandchild");

    // A missing cwd fails the start.
    assert!(
        platform
            .spawn(ProcessSpec {
                executable: "/bin/sh".into(),
                args: vec!["-c".into(), "true".into()],
                cwd: cwd.join("dim-missing-cwd-for-spawn"),
                env: vec![],
            })
            .is_err()
    );
}
