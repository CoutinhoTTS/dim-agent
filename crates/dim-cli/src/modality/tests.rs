//! The Modality commands against a scripted daemon. The CLI speaks only the
//! protocol, so these tests link no App Server; the product composition is
//! proven in `crates/dimcode/tests/cli_flow/`.

use super::{Command, run};
use crate::args::{Action, Options};
use crate::client::Client;
use dim_client::{ConnectionConfig, Server};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write as _};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long one scripted step waits before it fails instead of hanging.
const PATIENCE: Duration = Duration::from_secs(10);

/// The capability read every Modality command needs.
const CAPABILITIES: &str = "modality.capabilities";

/// The features every daemon advertises, so the handshake passes them.
const BASE_FEATURES: [&str; 2] = ["session.create", "session.history"];

/// The daemon's hello, advertising `features`.
fn hello_result(features: &[&str]) -> Value {
    let features: Vec<&str> = BASE_FEATURES
        .iter()
        .copied()
        .chain(features.iter().copied())
        .collect();
    json!({
        "instanceId": "scripted",
        "serverVersion": "test",
        "protocolVersion": 3,
        "features": features,
        "daemonProfile": {"kind": "product"},
    })
}

/// One scripted daemon and the home its socket sits in.
pub(super) struct Script {
    home: tempfile::TempDir,
    socket: PathBuf,
    daemon: Option<std::thread::JoinHandle<()>>,
}

impl Script {
    /// Binds the socket, then runs `flow` on the one connection the CLI
    /// opens. Binding first means a client can connect at once.
    pub(super) fn start(flow: impl FnOnce(&mut Wire) + Send + 'static) -> Self {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let daemon = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the CLI connected");
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            flow(&mut Wire(BufReader::new(stream)));
        });
        Self {
            home,
            socket,
            daemon: Some(daemon),
        }
    }

    /// A connected client, past the handshake every CLI action starts with.
    pub(super) async fn client(&self) -> Client {
        let config = ConnectionConfig {
            endpoint: self.socket.clone(),
            server: Server {
                program: PathBuf::from("dimcode"),
                embedded: false,
            },
            profile: None,
            client_kind: "cli",
            client_name: "dimcode",
        };
        Client::connect(&config).await.unwrap()
    }

    pub(super) fn home(&self) -> &Path {
        self.home.path()
    }

    /// Surfaces a scripted assertion that failed in the daemon's thread.
    pub(super) fn join(&mut self) {
        if let Some(daemon) = self.daemon.take() {
            daemon.join().unwrap();
        }
    }
}

/// The daemon's side of the connection: JSON-RPC frames, one line each.
pub(super) struct Wire(BufReader<std::os::unix::net::UnixStream>);

impl Wire {
    pub(super) fn frame(&mut self) -> Value {
        self.next_frame().expect("the CLI closed before a frame")
    }

    /// The next request, or `None` when the CLI closed first.
    pub(super) fn next_frame(&mut self) -> Option<Value> {
        let deadline = Instant::now() + PATIENCE;
        let mut line = String::new();
        loop {
            match self.0.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) => return Some(serde_json::from_str(&line).unwrap()),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    assert!(
                        Instant::now() < deadline,
                        "the CLI sent nothing within {PATIENCE:?}"
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => return None,
                Err(error) => panic!("{error}"),
            }
        }
    }

    pub(super) fn reply(&mut self, request: &Value, result: Value) {
        self.send(json!({"jsonrpc": "2.0", "id": request["id"], "result": result}));
    }

    /// An error reply carrying `app_error`, as the owner's refusals do.
    pub(super) fn error(&mut self, request: &Value, app_error: &str, message: &str) {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "error": {
                "code": -32000,
                "message": message,
                "data": {"appError": app_error},
            },
        }));
    }

    pub(super) fn send(&mut self, value: Value) {
        let written = writeln!(self.0.get_mut(), "{value}").and_then(|()| self.0.get_mut().flush());
        if let Err(error) = written {
            assert!(
                matches!(
                    error.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                ),
                "{error}"
            );
        }
    }

    /// Answers the handshake, advertising `features`.
    pub(super) fn hello(&mut self, features: &[&str]) {
        let hello = self.frame();
        assert_eq!(hello["method"], "hello");
        self.reply(&hello, hello_result(features));
    }
}

/// The arguments `dimcode ARGS...` in `home` are parsed from.
fn raw(home: &Path, args: &[&str]) -> Vec<String> {
    let mut arguments = vec![
        "--home".to_owned(),
        home.to_str().unwrap().to_owned(),
        "--cwd".to_owned(),
        home.to_str().unwrap().to_owned(),
    ];
    arguments.extend(args.iter().map(|argument| (*argument).to_owned()));
    arguments
}

/// The options `dimcode ARGS...` parses in `home`.
pub(super) fn parse(home: &Path, args: &[&str]) -> Options {
    Options::parse(raw(home, args).into_iter()).unwrap()
}

/// Runs the parsed Modality command, returning its exit code and stdout.
pub(super) async fn invoke(client: &mut Client, options: &Options) -> crate::Result<(i32, String)> {
    let Action::Modality(command) = &options.action else {
        panic!("not a Modality command");
    };
    let mut out = Vec::new();
    let code = run(client, options, command, &mut out).await?;
    Ok((code, String::from_utf8(out).unwrap()))
}

fn operation(category: &str, operation: &str, available: bool) -> Value {
    let reason = json!({"dimension": "adapter", "detail": "No image adapter in this build"});
    json!({
        "operation": operation,
        "key": format!("modality.{category}.{operation}"),
        "available": available,
        "reasons": if available { json!([]) } else { json!([reason]) },
        "default": null,
        "effective": {"providerId": "openai", "modelId": "gpt-image-2", "source": "automatic"},
        "options": [{"providerId": "openai", "modelId": "gpt-image-2", "available": true}],
    })
}

/// The snapshot one daemon answers, so a flow can reply with it.
fn snapshot(revision: &str) -> Value {
    json!({
        "revision": revision,
        "categories": [
            {
                "category": "image",
                "available": false,
                "reasons": [{"dimension": "adapter", "detail": "No image adapter in this build"}],
                "operations": [
                    operation("image", "generate", false),
                    operation("image", "understand", true),
                ],
            },
            {
                "category": "ocr",
                "available": false,
                "reasons": [{"dimension": "build", "detail": "No OCR helper in this build"}],
                "operations": [json!({
                    "operation": "recognize",
                    "key": "modality.ocr",
                    "available": false,
                    "reasons": [{"dimension": "build", "detail": "No OCR helper in this build"}],
                    "default": null,
                    "effective": null,
                    "options": [],
                })],
            },
        ],
    })
}

/// The table of the listing: a header and one tab-separated row per operation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_listing_renders_every_operation_with_its_reasons() {
    let mut script = Script::start(|wire| {
        wire.hello(&[CAPABILITIES]);
        let read = wire.frame();
        assert_eq!(read["method"], CAPABILITIES);
        wire.reply(&read, snapshot("native:user/modality:3"));
    });
    let mut client = script.client().await;
    let options = parse(script.home(), &["modality"]);
    let (code, out) = invoke(&mut client, &options).await.unwrap();
    assert_eq!(code, 0);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[0],
        "CATEGORY\tOPERATION\tAVAILABLE\tDEFAULT\tEFFECTIVE\tREASON"
    );
    assert_eq!(
        lines[1],
        "image\tgenerate\tno\t-\topenai/gpt-image-2 (automatic)\tadapter: No image adapter in this build"
    );
    assert_eq!(
        lines[2],
        "image\tunderstand\tyes\t-\topenai/gpt-image-2 (automatic)\t-"
    );
    assert_eq!(
        lines[3],
        "ocr\trecognize\tno\t-\t-\tbuild: No OCR helper in this build"
    );
    assert_eq!(lines.len(), 4);
    script.join();
}

/// `--json` is the reply the daemon sent, so a client of the snapshot reads it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_prints_the_snapshot_unchanged() {
    let mut script = Script::start(|wire| {
        wire.hello(&[CAPABILITIES]);
        let read = wire.frame();
        wire.reply(&read, snapshot("native:user/modality:3"));
    });
    let mut client = script.client().await;
    let options = parse(script.home(), &["modality", "--json"]);
    let (code, out) = invoke(&mut client, &options).await.unwrap();
    assert_eq!(code, 0);
    let printed: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(printed, snapshot("native:user/modality:3"));
    script.join();
}

/// `set-default` writes the revision it just read and reports the operation's
/// row from the reply, which is the next snapshot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_default_writes_the_revision_it_read() {
    let mut script = Script::start(|wire| {
        wire.hello(&[CAPABILITIES]);
        let read = wire.frame();
        assert_eq!(read["method"], CAPABILITIES);
        wire.reply(&read, snapshot("revision-1"));
        let write = wire.frame();
        assert_eq!(write["method"], "modality.configure");
        assert_eq!(
            write["params"],
            json!({
                "expectedRevision": "revision-1",
                "category": "image",
                "operation": "generate",
                "model": {"providerId": "openai", "modelId": "gpt-image-2"},
            })
        );
        wire.reply(&write, snapshot("revision-2"));
    });
    let mut client = script.client().await;
    let options = parse(
        script.home(),
        &[
            "modality",
            "set-default",
            "image.generate",
            "openai/gpt-image-2",
        ],
    );
    let (code, out) = invoke(&mut client, &options).await.unwrap();
    assert_eq!(code, 0);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "Set image.generate to openai/gpt-image-2.");
    assert!(lines[1].starts_with("image\tgenerate\t"), "{out}");
    script.join();
}

/// `--clear` sends a null model and no operation for a category-keyed
/// category.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_sends_a_null_model() {
    let mut script = Script::start(|wire| {
        wire.hello(&[CAPABILITIES]);
        let read = wire.frame();
        wire.reply(&read, snapshot("revision-1"));
        let write = wire.frame();
        assert_eq!(write["method"], "modality.configure");
        assert_eq!(
            write["params"],
            json!({
                "expectedRevision": "revision-1",
                "category": "image",
                "model": null,
            })
        );
        wire.reply(&write, snapshot("revision-2"));
    });
    let mut client = script.client().await;
    let options = parse(
        script.home(),
        &["modality", "set-default", "image", "--clear"],
    );
    let (code, out) = invoke(&mut client, &options).await.unwrap();
    assert_eq!(code, 0);
    assert!(out.starts_with("Cleared image.\n"), "{out}");
    script.join();
}

/// A stale revision is reread once, and the retry carries the new one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_conflict_is_reread_and_retried_once() {
    let mut script = Script::start(|wire| {
        wire.hello(&[CAPABILITIES]);
        let first = wire.frame();
        wire.reply(&first, snapshot("revision-1"));
        let write = wire.frame();
        assert_eq!(write["params"]["expectedRevision"], "revision-1");
        wire.error(
            &write,
            "modality_conflict",
            "Modality defaults changed; read the current snapshot before saving again",
        );
        let reread = wire.frame();
        assert_eq!(reread["method"], CAPABILITIES);
        wire.reply(&reread, snapshot("revision-2"));
        let retry = wire.frame();
        assert_eq!(retry["method"], "modality.configure");
        assert_eq!(retry["params"]["expectedRevision"], "revision-2");
        wire.reply(&retry, snapshot("revision-3"));
    });
    let mut client = script.client().await;
    let options = parse(
        script.home(),
        &[
            "modality",
            "set-default",
            "image.generate",
            "openai/gpt-image-2",
        ],
    );
    let (code, _) = invoke(&mut client, &options).await.unwrap();
    assert_eq!(code, 0);
    script.join();
}

/// A second conflict is the command's failure, with the owner's message.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_conflict_fails_with_the_owner_message() {
    let mut script = Script::start(|wire| {
        wire.hello(&[CAPABILITIES]);
        let first = wire.frame();
        wire.reply(&first, snapshot("revision-1"));
        let write = wire.frame();
        wire.error(
            &write,
            "modality_conflict",
            "Modality defaults changed; read the current snapshot before saving again",
        );
        let reread = wire.frame();
        wire.reply(&reread, snapshot("revision-2"));
        let retry = wire.frame();
        assert_eq!(retry["params"]["expectedRevision"], "revision-2");
        wire.error(
            &retry,
            "modality_conflict",
            "Modality defaults changed; read the current snapshot before saving again",
        );
    });
    let mut client = script.client().await;
    let options = parse(
        script.home(),
        &[
            "modality",
            "set-default",
            "image.generate",
            "openai/gpt-image-2",
        ],
    );
    let error = invoke(&mut client, &options)
        .await
        .expect_err("a second conflict fails");
    assert!(
        error.to_string().contains("Modality defaults changed"),
        "{error}"
    );
    script.join();
}

/// A daemon that does not advertise the read serves no Modality.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_daemon_without_modality_says_so() {
    let mut script = Script::start(|wire| {
        wire.hello(&[]);
        assert!(
            wire.next_frame().is_none(),
            "a command for a daemon without Modality calls nothing"
        );
    });
    let mut client = script.client().await;
    let options = parse(script.home(), &["modality"]);
    let error = invoke(&mut client, &options)
        .await
        .expect_err("no Modality");
    assert_eq!(error.to_string(), "This daemon serves no Modality.");
    drop(client);
    script.join();
}

/// The parsed command lines: each shape reaches its command, and a word the
/// namespace does not have is a usage error.
#[test]
fn the_command_lines_parse_to_their_commands() {
    let home = tempfile::tempdir().unwrap();
    let command = |args: &[&str]| match parse(home.path(), args).action {
        Action::Modality(command) => Some(command),
        _ => None,
    };
    assert!(matches!(command(&["modality"]), Some(Command::List)));
    match command(&[
        "modality",
        "set-default",
        "image.edit",
        "openai/gpt-image-2",
    ]) {
        Some(Command::SetDefault {
            operation,
            model,
            clear,
        }) => {
            assert_eq!(operation, "image.edit");
            assert_eq!(model.as_deref(), Some("openai/gpt-image-2"));
            assert!(!clear);
        }
        _ => panic!("set-default did not parse"),
    }
    match command(&["modality", "set-default", "video", "--clear"]) {
        Some(Command::SetDefault {
            operation,
            model,
            clear,
        }) => {
            assert_eq!(operation, "video");
            assert!(model.is_none());
            assert!(clear);
        }
        _ => panic!("--clear did not parse"),
    }
    assert!(Options::parse(raw(home.path(), &["modality", "frobnicate"]).into_iter()).is_err());
    assert!(Options::parse(raw(home.path(), &["modality", "--json"]).into_iter()).is_ok());
    assert!(Options::parse(raw(home.path(), &["models", "openai", "--json"]).into_iter()).is_err());
    assert!(
        Options::parse(
            raw(
                home.path(),
                &["modality", "set-default", "image", "--clear", "x/y"]
            )
            .into_iter()
        )
        .is_err()
    );
}
