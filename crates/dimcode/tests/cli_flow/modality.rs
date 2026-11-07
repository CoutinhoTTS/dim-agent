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

/// One operation of a `modality.capabilities` reply.
fn listed<'a>(snapshot: &'a Value, category: &str, name: &str) -> &'a Value {
    snapshot["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["category"] == category)
        .and_then(|entry| {
            entry["operations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|operation| operation["operation"] == name)
        })
        .unwrap()
}

/// `set-default` against the product daemon: the OCR operation's host target
/// is the model this build's snapshot accepts without a provider.
#[test]
fn product_set_default_writes_and_clears_the_ocr_default() {
    let home = Home::new();
    let listing = home
        .command()
        .args(["modality", "--json"])
        .output()
        .unwrap();
    assert!(
        listing.status.success(),
        "{}",
        String::from_utf8_lossy(&listing.stderr)
    );
    let snapshot: Value = serde_json::from_slice(&listing.stdout).unwrap();
    let recognize = listed(&snapshot, "ocr", "recognize");
    let option = &recognize["options"][0];
    let model = format!(
        "{}/{}",
        option["providerId"].as_str().unwrap(),
        option["modelId"].as_str().unwrap()
    );
    let output = home
        .command()
        .args(["modality", "set-default", "ocr.recognize", &model])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .starts_with(&format!("Set ocr.recognize to {model}.")),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let listing = home
        .command()
        .args(["modality", "--json"])
        .output()
        .unwrap();
    let written: Value = serde_json::from_slice(&listing.stdout).unwrap();
    let recognize = listed(&written, "ocr", "recognize");
    assert_eq!(
        recognize["default"]["providerId"], option["providerId"],
        "{recognize}"
    );
    assert_eq!(
        recognize["default"]["modelId"], option["modelId"],
        "{recognize}"
    );
    assert_eq!(recognize["effective"]["source"], "default", "{recognize}");

    let output = home
        .command()
        .args(["modality", "set-default", "ocr.recognize", "--clear"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("Cleared ocr.recognize."));
    let listing = home
        .command()
        .args(["modality", "--json"])
        .output()
        .unwrap();
    let cleared: Value = serde_json::from_slice(&listing.stdout).unwrap();
    let recognize = listed(&cleared, "ocr", "recognize");
    assert_eq!(recognize["default"], Value::Null, "{recognize}");
}
