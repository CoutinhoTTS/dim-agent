//! `dimcode modality` and `set-default`: the capability snapshot and one
//! default write over the public wire (Modality A.5).
use super::*;

fn hello(wire: &mut Wire) {
    wire.hello(&["modality.capabilities", "modality.configure"]);
}

fn operation(category: &str, operation: &str, available: bool) -> Value {
    let reasons = if available {
        json!([])
    } else {
        json!([{"dimension": "adapter", "detail": "No image adapter in this build"}])
    };
    json!({
        "operation": operation,
        "key": format!("modality.{category}.{operation}"),
        "available": available,
        "reasons": reasons,
        "default": null,
        "effective": {"providerId": "openai", "modelId": "gpt-image-2", "source": "automatic"},
        "options": [{"providerId": "openai", "modelId": "gpt-image-2", "available": true}],
    })
}

fn snapshot(revision: &str) -> Value {
    json!({
        "revision": revision,
        "categories": [
            {
                "category": "image",
                "available": false,
                "reasons": [{"dimension": "adapter", "detail": "No image adapter in this build"}],
                "operations": [operation("image", "generate", false), operation("image", "understand", true)],
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

#[test]
fn modality_listing_renders_the_operation_table() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        hello(wire);
        let request = wire.read("modality.capabilities");
        assert_eq!(request["params"], json!({}));
        wire.reply(&request, snapshot("revision-1"));
    });
    let output = home.command().args(["modality"]).output().unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "CATEGORY\tOPERATION\tAVAILABLE\tDEFAULT\tEFFECTIVE\tREASON\n\
         image\tgenerate\tno\t-\topenai/gpt-image-2 (automatic)\tadapter: No image adapter in this build\n\
         image\tunderstand\tyes\t-\topenai/gpt-image-2 (automatic)\t-\n\
         ocr\trecognize\tno\t-\t-\tbuild: No OCR helper in this build\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn modality_json_prints_the_reply_unchanged() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        hello(wire);
        let request = wire.read("modality.capabilities");
        wire.reply(&request, snapshot("revision-1"));
    });
    let output = home
        .command()
        .args(["modality", "--json"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(printed, snapshot("revision-1"));
}

#[test]
fn set_default_writes_the_revision_it_read_and_retries_a_conflict() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        hello(wire);
        let read = wire.read("modality.capabilities");
        wire.reply(&read, snapshot("revision-1"));
        let write = wire.read("modality.configure");
        assert_eq!(
            write["params"],
            json!({
                "expectedRevision": "revision-1",
                "category": "image",
                "operation": "generate",
                "model": {"providerId": "openai", "modelId": "gpt-image-2"},
            })
        );
        wire.send(json!({"jsonrpc":"2.0","id":write["id"],"error":{
            "code":-32000,
            "message":"Modality defaults changed; read the current snapshot before saving again",
            "data":{"appError":"modality_conflict"}}}));
        let reread = wire.read("modality.capabilities");
        wire.reply(&reread, snapshot("revision-2"));
        let retry = wire.read("modality.configure");
        assert_eq!(retry["params"]["expectedRevision"], "revision-2");
        wire.reply(&retry, snapshot("revision-3"));
    });
    let output = home
        .command()
        .args([
            "modality",
            "set-default",
            "image.generate",
            "openai/gpt-image-2",
        ])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Set image.generate to openai/gpt-image-2.\n\
         image\tgenerate\tno\t-\topenai/gpt-image-2 (automatic)\tadapter: No image adapter in this build\n"
    );
}

#[test]
fn set_default_clear_sends_a_null_model() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        hello(wire);
        let read = wire.read("modality.capabilities");
        wire.reply(&read, snapshot("revision-1"));
        let write = wire.read("modality.configure");
        assert_eq!(
            write["params"],
            json!({
                "expectedRevision": "revision-1",
                "category": "ocr",
                "model": null,
            })
        );
        wire.reply(&write, snapshot("revision-2"));
    });
    let output = home
        .command()
        .args(["modality", "set-default", "ocr", "--clear"])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("Cleared ocr.\nocr\trecognize\t"),
    );
}

#[test]
fn a_daemon_without_modality_says_so_and_exits_one() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        wire.hello(&[]);
        assert!(
            wire.trailing_request().is_none(),
            "a command for a daemon without Modality calls nothing"
        );
    });
    let output = home.command().args(["modality"]).output().unwrap();
    daemon.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "Error: This daemon serves no Modality.\n"
    );
}

/// The record a daemon answers with (A.4), in `state`.
fn record(task_id: &str, state: &str) -> Value {
    json!({
        "version": 1,
        "task_id": task_id,
        "client_task_id": null,
        "revision": 1,
        "category": "ocr",
        "operation": "recognize",
        "executor": {
            "kind": "host",
            "node": null,
            "target": {"providerId": "local", "modelId": "apple-vision"},
        },
        "params": {"outputMode": "text"},
        "state": state,
        "remote_handle": null,
        "progress": null,
        "result": null,
        "error": null,
        "remote_cancel": null,
        "created_at": "2026-11-07T00:00:00Z",
        "updated_at": "2026-11-07T00:00:00Z",
        "finished_at": null,
    })
}

/// A result transfer is stopped by the first Ctrl+C: no target and no
/// temporary remain, the id is printed, and the command exits 130.
#[test]
fn the_first_interrupt_stops_a_result_transfer() {
    use std::sync::mpsc;
    let home = Home::new();
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("modality-transfer-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&out).unwrap();
    let (waiting, waiting_rx) = mpsc::channel();
    let daemon = home.daemon(move |wire| {
        wire.hello(&["modality.capabilities", "modality.task.submit"]);
        let submit = wire.read("modality.task.submit");
        wire.reply(&submit, json!({"task": record("ocr-3", "queued")}));
        let wait = wire.read("modality.task.wait");
        let mut done = record("ocr-3", "succeeded");
        done["result"] = json!({"files": [
            {"name": "image-1.png", "media_type": "image/png", "bytes": 4, "sha256": "00"},
        ], "data": null});
        wire.reply(&wait, json!({"task": done}));
        let result = wire.read("modality.task.result");
        assert_eq!(result["params"]["name"], "image-1.png");
        waiting.send(()).unwrap();
        // Left unanswered: the interrupt arrives inside the transfer.
        assert!(wire.trailing_request().is_none());
    });
    let child = home
        .command()
        .args(["image", "generate", "a cat", "--out", out.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    waiting_rx.recv_timeout(HANG_GUARD).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let output = child.wait_with_output().unwrap();
    daemon.join().unwrap();
    assert_eq!(output.status.code(), Some(130));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "ocr-3\n");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(
            "Stopped; the task keeps its results. Fetch them with: \
             dimcode modality task wait ocr-3 --out <dir>"
        ),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_dir(&out).unwrap().count(),
        0,
        "neither the target nor a temporary exists"
    );
    fs::remove_dir_all(&out).unwrap();
}
