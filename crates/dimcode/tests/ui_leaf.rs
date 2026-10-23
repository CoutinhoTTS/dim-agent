#![cfg(unix)]
//! End-to-end evidence for the daemon-owned UiLeaf: the daemon starts a
//! leaf process, the leaf hosts its rutis plugin, the TUI attaches its
//! protocol v3 connection, `ui.status` combines both, `ui.close` ends the
//! process, and `ui.open {replace}` advances a generation.

use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    os::{
        fd::RawFd,
        unix::{
            fs::{DirBuilderExt, PermissionsExt},
            net::UnixStream,
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Barrier},
    time::{Duration, Instant},
};

struct Daemon {
    child: Child,
    socket: PathBuf,
    log: PathBuf,
    home: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Pty {
    master: RawFd,
    slave: RawFd,
    path: String,
}

impl Pty {
    fn new() -> Self {
        let mut master = -1;
        let mut slave = -1;
        let result = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(result, 0, "openpty: {}", std::io::Error::last_os_error());
        let size = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(master, libc::TIOCSWINSZ, &size);
        }
        let name = unsafe { libc::ttyname(slave) };
        assert!(!name.is_null(), "ttyname");
        let path = unsafe { std::ffi::CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        Self {
            master,
            slave,
            path,
        }
    }

    fn write(&self, bytes: &[u8]) {
        let written = unsafe {
            libc::write(
                self.master,
                bytes.as_ptr().cast::<libc::c_void>(),
                bytes.len(),
            )
        };
        assert_eq!(written, bytes.len() as isize, "write terminal");
    }

    fn read_until(&self, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut output = Vec::new();
        while Instant::now() < deadline {
            let mut poll = libc::pollfd {
                fd: self.master,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut poll, 1, 50) };
            if ready > 0 {
                let mut buffer = [0u8; 8192];
                let read = unsafe {
                    libc::read(
                        self.master,
                        buffer.as_mut_ptr().cast::<libc::c_void>(),
                        buffer.len(),
                    )
                };
                if read > 0 {
                    output.extend_from_slice(&buffer[..read as usize]);
                    let text = String::from_utf8_lossy(&output);
                    if text.contains(needle) {
                        return text.into_owned();
                    }
                }
            }
        }
        panic!(
            "terminal output never contained {needle:?}: {}",
            String::from_utf8_lossy(&output)
        );
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.master);
            libc::close(self.slave);
        }
    }
}

struct Rpc {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next: u64,
}

impl Rpc {
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("daemon socket");
        let reader = BufReader::new(stream.try_clone().expect("socket clone"));
        Self {
            stream,
            reader,
            next: 0,
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        writeln!(self.stream, "{request}").expect("write request");
        self.stream.flush().expect("flush request");
        loop {
            let mut line = String::new();
            self.reader.read_line(&mut line).expect("read response");
            assert!(!line.is_empty(), "daemon closed while waiting for {method}");
            let message: Value = serde_json::from_str(&line).expect("json response");
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                return message;
            }
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let response = self.call(method, params);
        assert!(
            response.get("error").is_none(),
            "{method} failed: {response}"
        );
        response["result"].clone()
    }
}

fn private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

fn wait_socket(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if UnixStream::connect(path).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("daemon socket did not appear at {}", path.display());
}

fn wait_online(rpc: &mut Rpc, ui_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let status = rpc.ok("ui.status", json!({}));
        if let Some(instance) = status["instances"]
            .as_array()
            .and_then(|instances| instances.iter().find(|instance| instance["uiId"] == ui_id))
            && instance["state"] == "online"
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "leaf {ui_id} never became online: {status}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_gone(rpc: &mut Rpc, ui_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = rpc.ok("ui.status", json!({}));
        let listed = status["instances"]
            .as_array()
            .is_some_and(|instances| instances.iter().any(|instance| instance["uiId"] == ui_id));
        if !listed {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "leaf {ui_id} was not removed: {status}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn gone(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == -1 }
}

fn wait_pid_gone(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !gone(pid) {
        assert!(Instant::now() < deadline, "leaf process {pid} survived");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn a_leaf_starts_replaces_and_is_cleaned_up() {
    let root = PathBuf::from("/tmp").join(format!("dim-ui-leaf-{}", uuid::Uuid::new_v4()));
    let home = root.clone();
    let rust_home = root.join("rust");
    private_dir(&rust_home).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_dimcode"));
    let model_config =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../dim-agent-aimux/examples/offline.json");
    let socket = rust_home.join("daemon.sock");
    let log = root.join("daemon.log");
    let stderr = File::create(&log).unwrap();
    let child = Command::new(&binary)
        .arg("app-server")
        .arg("--model-config")
        .arg(&model_config)
        .arg("--unix")
        .arg(&socket)
        .arg("--home")
        .arg(&rust_home)
        .arg("--no-ws")
        .arg("--ui-launcher")
        .arg(&binary)
        .arg("--ui-launcher-arg")
        .arg("ui-leaf")
        .env("DIMCODE_HOME", &home)
        .env("DIMCODE_UPDATE_CHECK", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .unwrap();
    let mut daemon = Daemon {
        child,
        socket: socket.clone(),
        log,
        home: root.clone(),
    };
    wait_socket(&socket);

    // The terminal client's Sessions are the ones `session.summaries`
    // reports to the leaf TUI; create one through the same executable.
    let output = Command::new(&binary)
        .args([
            "--server",
            binary.to_str().unwrap(),
            "--model-config",
            model_config.to_str().unwrap(),
            "exec",
            "-p",
            "hello",
            "--json",
        ])
        .env("DIMCODE_HOME", &home)
        .current_dir(&root)
        .stdin(Stdio::null())
        .output()
        .expect("create session");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let session = stdout
        .lines()
        .chain(stderr.lines())
        .find_map(|line| line.strip_prefix("session: "))
        .unwrap_or_else(|| panic!("no session in CLI output: {stdout}\n{stderr}"))
        .to_owned();

    let mut rpc = Rpc::connect(&socket);
    let hello = rpc.ok(
        "hello",
        json!({"protocolVersion": 3, "clientKind": "cli", "clientName": "ui-leaf-test"}),
    );
    assert_eq!(hello["protocolVersion"], 3);
    rpc.ok("session.attach", json!({"sessionId": session}));

    let first_pty = Pty::new();
    let first = rpc.ok(
        "ui.open",
        json!({"sessionId": session, "kind": "tui", "tty": first_pty.path}),
    );
    let first_id = first["uiId"].as_str().unwrap().to_owned();
    let first_pid = first["pid"].as_u64().unwrap() as u32;
    wait_online(&mut rpc, &first_id);
    // The TUI holds the caller's terminal: typed input reaches the leaf and
    // is echoed by the attached TUI, not just written to a detached pipe.
    first_pty.read_until("Welcome to DIM");
    std::thread::sleep(Duration::from_secs(1));
    first_pty.write(b"leaf-input-proof");
    first_pty.read_until("leaf-input-proof");

    // Two callers observe the same old generation. Exactly one may claim the
    // replacement; the other must be refused and must not leave a provisional
    // leaf behind.
    let replacement_a_pty = Pty::new();
    let replacement_b_pty = Pty::new();
    let mut rpc_a = Rpc::connect(&socket);
    rpc_a.ok(
        "hello",
        json!({"protocolVersion": 3, "clientKind": "cli", "clientName": "ui-leaf-a"}),
    );
    rpc_a.ok("session.attach", json!({"sessionId": session}));
    let mut rpc_b = Rpc::connect(&socket);
    rpc_b.ok(
        "hello",
        json!({"protocolVersion": 3, "clientKind": "cli", "clientName": "ui-leaf-b"}),
    );
    rpc_b.ok("session.attach", json!({"sessionId": session}));
    let barrier = Arc::new(Barrier::new(3));
    let launch = |mut rpc: Rpc, tty: String| {
        let barrier = barrier.clone();
        let session = session.clone();
        let first_id = first_id.clone();
        let generation = first["generation"].clone();
        std::thread::spawn(move || {
            barrier.wait();
            let response = rpc.call(
                "ui.open",
                json!({
                    "sessionId": session,
                    "kind": "tui",
                    "tty": tty,
                    "replace": first_id,
                    "expectedGeneration": generation,
                }),
            );
            (response, rpc)
        })
    };
    let a = launch(rpc_a, replacement_a_pty.path.clone());
    let b = launch(rpc_b, replacement_b_pty.path.clone());
    barrier.wait();
    let results = [a.join().unwrap(), b.join().unwrap()];
    let mut winner = None;
    let mut refused = 0;
    for (response, _) in results {
        if response.get("error").is_none() {
            assert!(
                winner.is_none(),
                "both replacement launches won: {response}"
            );
            winner = Some(response["result"].clone());
        } else {
            refused += 1;
        }
    }
    assert_eq!(refused, 1, "exactly one replacement must lose");
    let second = winner.expect("one replacement succeeds");
    let second_id = second["uiId"].as_str().unwrap().to_owned();
    let second_pid = second["pid"].as_u64().unwrap() as u32;
    assert_eq!(
        second["generation"].as_u64(),
        first["generation"]
            .as_u64()
            .map(|generation| generation + 1)
    );
    wait_online(&mut rpc, &second_id);
    wait_gone(&mut rpc, &first_id);
    wait_pid_gone(first_pid);
    let status = rpc.ok("ui.status", json!({}));
    assert_eq!(
        status["instances"].as_array().map(Vec::len),
        Some(1),
        "a losing replacement left a registration: {status}"
    );
    assert_eq!(
        status["instances"][0]["uiId"].as_str(),
        Some(second_id.as_str())
    );

    // A stopped UI cannot answer the advisory shutdown; the leaf process is
    // still killed after the grace period.
    unsafe {
        libc::kill(second_pid as libc::pid_t, libc::SIGSTOP);
    }
    rpc.ok("ui.close", json!({"uiId": second_id}));
    wait_gone(&mut rpc, &second_id);
    wait_pid_gone(second_pid);

    // Daemon shutdown stops a leaf that is still online and leaves no
    // process behind.
    let third_pty = Pty::new();
    let third = rpc.ok(
        "ui.open",
        json!({"sessionId": session, "kind": "tui", "tty": third_pty.path}),
    );
    let third_id = third["uiId"].as_str().unwrap().to_owned();
    let third_pid = third["pid"].as_u64().unwrap() as u32;
    wait_online(&mut rpc, &third_id);
    let _ = rpc.call("daemon.shutdown", json!({}));
    let deadline = Instant::now() + Duration::from_secs(10);
    while daemon.child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        daemon.child.try_wait().unwrap().is_some(),
        "daemon did not stop; log: {}",
        std::fs::read_to_string(&daemon.log).unwrap_or_default()
    );
    wait_pid_gone(third_pid);
    let _ = &daemon.home;
    let _ = &daemon.socket;
}
