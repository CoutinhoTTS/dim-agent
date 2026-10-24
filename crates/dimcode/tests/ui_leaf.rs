#![cfg(unix)]
//! End-to-end evidence for UI leaves (Tui design §3.6): the daemon starts a
//! leaf node and hosts `Tui` in it, the TUI attaches its protocol v3
//! connection, `ui.close` unloads it and ends the process, replacements
//! advance a generation, the opener's disconnect closes its leaves, and
//! daemon shutdown restores the terminal and leaves no process.

use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    os::{
        fd::{FromRawFd, OwnedFd, RawFd},
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

    fn local_flags(&self) -> libc::tcflag_t {
        let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
        assert_eq!(
            unsafe { libc::tcgetattr(self.slave, &mut termios) },
            0,
            "tcgetattr"
        );
        termios.c_lflag
    }

    /// Reads whatever the terminal shows for `duration`, so a TUI never
    /// blocks on a full buffer while the test waits for something else.
    fn drain(&self, duration: Duration) -> String {
        let deadline = Instant::now() + duration;
        let mut output = Vec::new();
        while Instant::now() < deadline {
            let mut poll = libc::pollfd {
                fd: self.master,
                events: libc::POLLIN,
                revents: 0,
            };
            if unsafe { libc::poll(&mut poll, 1, 20) } > 0 {
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
                }
            }
        }
        String::from_utf8_lossy(&output).into_owned()
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

/// The leaf process of `ui_id`, found by its `--leaf-peer` argument.
fn leaf_pid(ui_id: &str) -> Option<u32> {
    let output = Command::new("pgrep")
        .args(["-f", &format!("ui-leaf --leaf-peer {ui_id}")])
        .output()
        .expect("pgrep");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .and_then(|line| line.trim().parse().ok())
}

fn wait_leaf_gone(ui_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while let Some(pid) = leaf_pid(ui_id) {
        assert!(Instant::now() < deadline, "leaf process {pid} survived");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn instance(rpc: &mut Rpc, ui_id: &str) -> Option<Value> {
    let status = rpc.ok("ui.status", json!({}));
    status["instances"]
        .as_array()?
        .iter()
        .find(|instance| instance["uiId"] == ui_id)
        .cloned()
}

fn open(rpc: &mut Rpc, session: &str, pty: &Pty, root: &Path, extra: Value) -> Value {
    let mut params = json!({
        "sessionId": session,
        "kind": "tui",
        "tty": pty.path,
        "cwd": root,
        "home": root,
    });
    for (key, value) in extra.as_object().into_iter().flatten() {
        params[key] = value.clone();
    }
    rpc.ok("ui.open", params)
}

#[test]
fn normal_dimcode_keeps_the_shell_away_until_the_leaf_leaves() {
    let root = PathBuf::from("/tmp").join(format!("dim-normal-leaf-{}", uuid::Uuid::new_v4()));
    let home = root.join("home");
    private_dir(&home).unwrap();
    let assets = root.join("web");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(
        assets.join("index.html"),
        "<!doctype html><title>Dim</title>",
    )
    .unwrap();
    std::fs::write(
        assets.join("dimcode-web.json"),
        r#"{"version":"0.0.1","protocolVersion":3,"sourceCommit":"test"}"#,
    )
    .unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_dimcode"));
    let model_config =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../dim-agent-aimux/examples/offline.json");
    let socket = home.join("rust/daemon.sock");
    let pty = Pty::new();
    let stdio = || {
        let fd = unsafe { libc::dup(pty.slave) };
        assert!(fd >= 0, "dup terminal");
        Stdio::from(unsafe { OwnedFd::from_raw_fd(fd) })
    };
    let script = "\"$0\" --model-config \"$1\"; code=$?; printf 'SHELL-RETURNED:%s\\n' \"$code\"; exit \"$code\"";
    let mut child = Command::new("sh")
        .args([
            "-c",
            script,
            binary.to_str().unwrap(),
            model_config.to_str().unwrap(),
        ])
        .env("DIMCODE_HOME", &home)
        .env("DIMCODE_WEB_ASSETS", &assets)
        .env("DIMCODE_OAUTH_MANUAL", "1")
        .env("DIMCODE_UPDATE_CHECK", "0")
        .env("LC_ALL", "C")
        .env("TERM", "xterm-256color")
        .stdin(stdio())
        .stdout(stdio())
        .stderr(stdio())
        .spawn()
        .expect("normal dimcode");
    let welcome = pty.read_until("Welcome to DIM");
    assert!(
        !welcome.contains("SHELL-RETURNED"),
        "the shell returned before the leaf left"
    );
    wait_socket(&socket);
    let mut rpc = Rpc::connect(&socket);
    rpc.ok(
        "hello",
        json!({"protocolVersion": 3, "clientKind": "cli", "clientName": "normal-leaf-test"}),
    );
    let status = rpc.ok("ui.status", json!({}));
    let instances = status["instances"].as_array().expect("instances");
    assert_eq!(instances.len(), 1, "{status}");
    let ui_id = instances[0]["uiId"].as_str().unwrap().to_owned();
    let first_session = instances[0]["sessionId"].as_str().unwrap().to_owned();
    // `/web` toggles the daemon switch and leaves the leaf TUI in place.
    pty.write(b"/web\r");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = rpc.ok("web.status", json!({}));
        if status["enabled"] == true && status["open"] == true {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Web switch did not start: {status}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    pty.write(b"/web\r");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = rpc.ok("web.status", json!({}));
        if status["enabled"] == false && status["open"] == false {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Web switch did not stop: {status}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    pty.write(b"/new\r");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = rpc.ok("ui.status", json!({}));
        let instances = status["instances"].as_array().expect("instances");
        assert_eq!(
            instances.len(),
            1,
            "a Session switch added an interface: {status}"
        );
        let instance = &instances[0];
        if instance["uiId"] == ui_id.as_str()
            && instance["sessionId"].as_str() != Some(first_session.as_str())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Session switch did not reattach: {status}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    pty.write(b"/exit\r");
    pty.read_until("SHELL-RETURNED:0");
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        child.try_wait().unwrap().is_some(),
        "dimcode did not return to the shell"
    );
    let _ = rpc.call("daemon.shutdown", json!({}));
    let _ = &pty;
    let _ = std::fs::remove_dir_all(&root);
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
        .arg("--ui-leaf-program")
        .arg(&binary)
        .env("DIMCODE_HOME", &home)
        .env("DIMCODE_UPDATE_CHECK", "0")
        .env("LC_ALL", "C")
        .env("TERM", "xterm-256color")
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

    // Configuration that is not a terminal is refused at the boundary.
    let refused = rpc.call(
        "ui.open",
        json!({"sessionId": session, "kind": "tui", "tty": "relative", "cwd": root, "home": root}),
    );
    assert_eq!(
        refused["error"]["data"]["appError"], "ui_tty_required",
        "{refused}"
    );

    let first_pty = Pty::new();
    let first = open(&mut rpc, &session, &first_pty, &root, json!({}));
    let first_id = first["uiId"].as_str().unwrap().to_owned();
    wait_online(&mut rpc, &first_id);
    // The TUI holds the caller's terminal: typed input reaches the leaf and
    // is echoed by the attached TUI, not just written to a detached pipe.
    first_pty.read_until("Welcome to DIM");
    first_pty.write(b"leaf-input-proof");
    first_pty.read_until("leaf-input-proof");

    // Two callers observe the same old generation. Exactly one may claim the
    // replacement; the other is refused and leaves no leaf behind.
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
        let root = root.clone();
        std::thread::spawn(move || {
            barrier.wait();
            let response = rpc.call(
                "ui.open",
                json!({
                    "sessionId": session,
                    "kind": "tui",
                    "tty": tty,
                    "cwd": root,
                    "home": root,
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
    let mut openers = Vec::new();
    for (response, rpc) in results {
        if response.get("error").is_none() {
            assert!(
                winner.is_none(),
                "both replacement launches won: {response}"
            );
            winner = Some(response["result"].clone());
            openers.push(rpc);
        } else {
            assert_eq!(
                response["error"]["data"]["appError"], "ui_generation",
                "{response}"
            );
        }
    }
    let second = winner.expect("one replacement succeeds");
    let second_id = second["uiId"].as_str().unwrap().to_owned();
    assert!(
        second["generation"].as_u64() > first["generation"].as_u64(),
        "{second}"
    );
    wait_online(&mut rpc, &second_id);
    wait_gone(&mut rpc, &first_id);
    wait_leaf_gone(&first_id);
    let status = rpc.ok("ui.status", json!({}));
    assert_eq!(
        status["instances"].as_array().map(Vec::len),
        Some(1),
        "a losing replacement left a registration: {status}"
    );

    // A stopped leaf cannot unload; it is still ended within the grace
    // periods.
    let second_pid = leaf_pid(&second_id).expect("second leaf runs");
    unsafe {
        libc::kill(second_pid as libc::pid_t, libc::SIGSTOP);
    }
    rpc.ok("ui.close", json!({"uiId": second_id}));
    wait_gone(&mut rpc, &second_id);
    wait_leaf_gone(&second_id);

    // A leaf belongs to its opener: when that connection ends, the leaf is
    // closed although its TUI is attached.
    let mut opener = Rpc::connect(&socket);
    opener.ok(
        "hello",
        json!({"protocolVersion": 3, "clientKind": "cli", "clientName": "ui-leaf-opener"}),
    );
    opener.ok("session.attach", json!({"sessionId": session}));
    let owned_pty = Pty::new();
    let owned = open(&mut opener, &session, &owned_pty, &root, json!({}));
    let owned_id = owned["uiId"].as_str().unwrap().to_owned();
    wait_online(&mut rpc, &owned_id);
    drop(opener);
    wait_gone(&mut rpc, &owned_id);
    wait_leaf_gone(&owned_id);
    drop(openers);

    // The advisory notice makes the TUI quit: it reports its exit code and
    // the leaf goes offline once its link stops.
    let third_pty = Pty::new();
    let third = open(&mut rpc, &session, &third_pty, &root, json!({}));
    let third_id = third["uiId"].as_str().unwrap().to_owned();
    wait_online(&mut rpc, &third_id);
    third_pty.read_until("Welcome to DIM");
    rpc.ok(
        "ui.shutdown",
        json!({"uiId": third_id, "reason": "manual shutdown"}),
    );
    third_pty.read_until("manual shutdown");
    // Keep reading the terminal so the TUI's last frames are not blocked
    // on a full buffer while it leaves.
    let deadline = Instant::now() + Duration::from_secs(10);
    let offline = loop {
        let shown = third_pty.drain(Duration::from_millis(100));
        let found = instance(&mut rpc, &third_id).expect("registered");
        if found["state"] == "offline" {
            break found;
        }
        assert!(
            Instant::now() < deadline,
            "the TUI did not leave: {found} {shown:?}"
        );
    };
    assert_eq!(offline["exitCode"], 0, "{offline}");
    wait_leaf_gone(&third_id);
    rpc.ok("ui.close", json!({"uiId": third_id}));

    // Daemon shutdown unloads a leaf that is still online: its terminal is
    // restored and no process is left.
    let fourth_pty = Pty::new();
    let fourth = open(&mut rpc, &session, &fourth_pty, &root, json!({}));
    let fourth_id = fourth["uiId"].as_str().unwrap().to_owned();
    wait_online(&mut rpc, &fourth_id);
    fourth_pty.read_until("Welcome to DIM");
    let raw_flags = fourth_pty.local_flags();
    assert_eq!(
        raw_flags & libc::ICANON as libc::tcflag_t,
        0,
        "TUI did not enter raw mode"
    );
    let _ = rpc.call("daemon.shutdown", json!({}));
    // A terminal emulator keeps reading; so does the test, or the restore
    // sequence would block on a full buffer.
    let deadline = Instant::now() + Duration::from_secs(10);
    while daemon.child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        fourth_pty.drain(Duration::from_millis(25));
    }
    assert!(
        daemon.child.try_wait().unwrap().is_some(),
        "daemon did not stop; log: {}",
        std::fs::read_to_string(&daemon.log).unwrap_or_default()
    );
    while leaf_pid(&fourth_id).is_some() && Instant::now() < deadline {
        fourth_pty.drain(Duration::from_millis(25));
    }
    wait_leaf_gone(&fourth_id);
    let restored = fourth_pty.local_flags();
    assert_ne!(
        restored & libc::ICANON as libc::tcflag_t,
        0,
        "terminal was left in raw mode"
    );
    assert_ne!(
        restored & libc::ECHO as libc::tcflag_t,
        0,
        "terminal echo was not restored"
    );
    assert_ne!(
        restored & libc::ISIG,
        0,
        "terminal signals were not restored"
    );
    let _ = &daemon.socket;
    let _ = std::fs::remove_dir_all(&daemon.home);
}
