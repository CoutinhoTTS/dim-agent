//! `dimcode models ...`: one Provider's models through its adapter, over the
//! public wire (cli-commands.md "The Models area").
use super::*;

fn hello(wire: &mut Wire) {
    wire.hello(&[
        "provider.management",
        "provider.models.set_enabled",
        "provider.models.edit",
        "provider.models.create",
        "provider.models.delete",
        "provider.models.reset",
        "provider.refresh",
    ]);
}

/// One model as `provider.management` reports it.
fn model(id: &str, name: &str, source: &str, enabled: bool, actions: &[&str]) -> Value {
    json!({
        "model": {
            "modelId": id,
            "displayName": name,
            "status": "active",
            "capabilities": {
                "modalities": ["text"],
                "toolCalling": true,
                "streaming": true,
                "structuredOutput": true,
                "vision": false,
                "audio": false,
                "reasoning": false,
                "contextWindow": 128000,
                "maxOutputTokens": 16384
            }
        },
        "source": source,
        "enabled": enabled,
        "endpoint": "openai-compatible",
        "actions": actions,
    })
}

/// The Provider as `provider.management` reports it.
fn provider(models: Value, version: u64) -> Value {
    json!({"providers": [{
        "providerId": "fixture",
        "displayName": "Fixture",
        "version": version,
        "actions": ["models.create", "models.edit", "models.delete", "models.reset",
                    "models.set_enabled", "preferences.update", "refresh"],
        "connection": {"driverKind": "new-api-router"},
        "models": models,
    }]})
}

/// The models a read reports: a catalog model, a user model, and an
/// override with its catalog entry.
fn models(flash_enabled: bool) -> Value {
    json!([
        model(
            "flash",
            "Flash",
            "default",
            flash_enabled,
            &["set_enabled", "edit"]
        ),
        model(
            "custom",
            "Custom",
            "user",
            true,
            &["set_enabled", "edit", "delete"]
        ),
        model(
            "tuned",
            "Tuned",
            "override",
            true,
            &["set_enabled", "edit", "reset"]
        ),
    ])
}

#[test]
fn models_area_verbs_write_through_the_management_methods() {
    // enable: the set is the complete enabled set with the model flipped.
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        hello(wire);
        let request = wire.read("provider.management");
        assert_eq!(request["params"], json!({"providerId": "fixture"}));
        wire.reply(&request, provider(models(false), 3));
        let request = wire.read("provider.models.set_enabled");
        assert_eq!(
            request["params"],
            json!({"providerId": "fixture", "expectedVersion": 3,
                "modelIds": ["flash", "custom", "tuned"]})
        );
        wire.reply(&request, json!({"models": []}));
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 4));
    });
    let output = home
        .command()
        .args(["models", "fixture", "enable", "flash"])
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
        "Enabled flash.\n"
    );

    // disable names the set without the model.
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        hello(wire);
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 3));
        let request = wire.read("provider.models.set_enabled");
        assert_eq!(
            request["params"],
            json!({"providerId": "fixture", "expectedVersion": 3,
                "modelIds": ["custom", "tuned"]})
        );
        wire.reply(&request, json!({"models": []}));
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(false), 4));
    });
    let output = home
        .command()
        .args(["models", "fixture", "disable", "flash"])
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
        "Disabled flash.\n"
    );

    // remove deletes a user model.
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        hello(wire);
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 3));
        let request = wire.read("provider.models.delete");
        assert_eq!(
            request["params"],
            json!({"providerId": "fixture", "expectedVersion": 3, "modelId": "custom"})
        );
        wire.reply(&request, json!({"models": []}));
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 4));
    });
    let output = home
        .command()
        .args(["models", "fixture", "remove", "custom"])
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
        "Removed custom.\n"
    );

    // remove resets an override to its catalog entry.
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        hello(wire);
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 3));
        let request = wire.read("provider.models.reset");
        assert_eq!(
            request["params"],
            json!({"providerId": "fixture", "expectedVersion": 3, "modelId": "tuned"})
        );
        wire.reply(&request, json!({"models": []}));
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 4));
    });
    let output = home
        .command()
        .args(["models", "fixture", "remove", "tuned"])
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
        "Reset tuned to its catalog entry.\n"
    );

    // add sends the document at the version read first.
    let home = Home::new();
    let path = home.0.join("added.json");
    fs::write(
        &path,
        r#"{"modelId":"new","displayName":"New model","status":"active","capabilities":{
            "modalities":["text"],"toolCalling":true,"streaming":true,
            "structuredOutput":true,"vision":false,"audio":false,"reasoning":false}}"#,
    )
    .unwrap();
    let daemon = home.daemon(|wire| {
        hello(wire);
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 3));
        let request = wire.read("provider.models.create");
        assert_eq!(request["params"]["providerId"], "fixture");
        assert_eq!(request["params"]["expectedVersion"], 3);
        assert_eq!(request["params"]["model"]["modelId"], "new");
        assert_eq!(request["params"]["model"]["displayName"], "New model");
        wire.reply(&request, json!({"models": []}));
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 4));
    });
    let output = home
        .command()
        .args(["models", "fixture", "add", path.to_str().unwrap()])
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
        "Added New model.\n"
    );
}

#[test]
fn models_area_set_writes_the_revision_it_read_and_only_the_changed_fields() {
    let home = Home::new();
    let path = home.0.join("flash.json");
    // The template the command builds from the read, with one field edited:
    // the write carries that field alone.
    fs::write(
        &path,
        json!({
            "displayName": "Renamed",
            "contextWindow": 128000,
            "maxOutputTokens": 16384,
            "toolCalling": true,
            "vision": false,
            "reasoning": {"supported": false}
        })
        .to_string(),
    )
    .unwrap();
    let daemon = home.daemon(|wire| {
        hello(wire);
        // The command reads the document, then the adapter reads the entry
        // it diffs the draft against.
        for _ in 0..2 {
            let request = wire.read("provider.management");
            wire.reply(&request, provider(models(true), 3));
        }
        let request = wire.read("provider.models.edit");
        assert_eq!(
            request["params"],
            json!({"providerId": "fixture", "expectedVersion": 3, "modelId": "flash",
                "edit": {"displayName": "Renamed"}})
        );
        wire.reply(&request, json!({"models": []}));
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 4));
    });
    let output = home
        .command()
        .args(["models", "fixture", "set", "flash", path.to_str().unwrap()])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Saved.\n");

    // A conflict writes nothing, and the user's text keeps its file.
    let home = Home::new();
    let path = home.0.join("flash.json");
    let kept = path.display().to_string();
    fs::write(
        &path,
        json!({
            "displayName": "Renamed",
            "contextWindow": 128000,
            "maxOutputTokens": 16384,
            "toolCalling": true,
            "vision": false,
            "reasoning": {"supported": false}
        })
        .to_string(),
    )
    .unwrap();
    let daemon = home.daemon(|wire| {
        hello(wire);
        for _ in 0..2 {
            let request = wire.read("provider.management");
            wire.reply(&request, provider(models(true), 3));
        }
        let request = wire.read("provider.models.edit");
        wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
            "code":-32000,
            "message":"Provider version changed; reread before retrying",
            "data":{"appError":"provider_conflict"}}}));
        // The conflict is classified by the version the fresh read reports.
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 4));
    });
    let output = home
        .command()
        .args(["models", "fixture", "set", "flash", path.to_str().unwrap()])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("The models changed since they were read; nothing was written. Your next edit opens the current list."),
        "{stderr}"
    );
    assert!(stderr.contains(&kept), "{stderr}");
}

#[test]
fn models_area_add_keeps_the_text_when_the_id_exists() {
    let home = Home::new();
    let path = home.0.join("added.json");
    let kept = path.display().to_string();
    fs::write(
        &path,
        r#"{"modelId":"flash","displayName":"Flash again","status":"active","capabilities":{
            "modalities":["text"],"toolCalling":true,"streaming":true,
            "structuredOutput":true,"vision":false,"audio":false,"reasoning":false}}"#,
    )
    .unwrap();
    let daemon = home.daemon(move |wire| {
        hello(wire);
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 3));
        // A duplicate id is the owner's own refusal: the version did not
        // move, so this is not a stale-revision conflict.
        let request = wire.read("provider.models.create");
        wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
            "code":-32000,
            "message":"Model already exists",
            "data":{"appError":"provider_conflict"}}}));
        let request = wire.read("provider.management");
        wire.reply(&request, provider(models(true), 3));
    });
    let output = home
        .command()
        .args(["models", "fixture", "add", path.to_str().unwrap()])
        .output()
        .unwrap();
    daemon.join().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Model already exists"), "{stderr}");
    assert!(stderr.contains(&kept), "{stderr}");
    assert!(output.stdout.is_empty());
}

/// A refresh the owner accepted is applied even when the read that reports
/// the area fails: the panel shows the refresh note and the read error, not
/// a refusal.
#[test]
fn tui_models_area_keeps_a_refresh_whose_read_fails() {
    let home = Home::new();
    let (shown, wait_shown) = std::sync::mpsc::channel();
    let daemon = home.daemon(move |wire| {
        hello(wire);
        wire.created();
        let attach = wire.read("session.attach");
        wire.reply(&attach, attach_record(&attach));
        let history = wire.read("session.history");
        wire.reply(
            &history,
            json!({"events":[],"through":8,"nextAfter":8,"done":true}),
        );
        for _ in 0..2 {
            let request = wire.read_any();
            match request["method"].as_str() {
                Some("account.providers") => wire.reply(&request, json!([])),
                Some("session.context.usage") => wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
                    "code":-32601,"message":"Unknown method","data":{"appError":"unknown_method"}}})),
                method => panic!("unexpected request {method:?}"),
            }
        }
        wait_shown.recv_timeout(PATIENCE).unwrap();
        let request = wire.read("provider.list");
        wire.reply(
            &request,
            json!({"providers":[{"providerId":"test","displayName":"Test","enabled":true}]}),
        );
        let request = wire.read("account.providers");
        wire.reply(&request, json!([]));
        let request = wire.read("provider.catalog");
        wire.reply(
            &request,
            json!({"catalog":{"version":3,"disabledModelIds":[],"models":[{
                "id":"flash","name":"Flash","capabilities":{
                    "modalities":["text"],"toolCalling":true,"streaming":true,
                    "structuredOutput":true,"vision":false,"audio":false,
                    "reasoning":false,"contextWindow":128000,"maxOutputTokens":16384}}]}}),
        );
        let area = json!({"providers": [{
            "providerId": "test",
            "displayName": "Test",
            "version": 3,
            "actions": ["models.create", "models.edit", "models.delete", "models.reset",
                        "models.set_enabled", "preferences.update", "refresh"],
            "connection": {"driverKind": "new-api-router"},
            "models": [{
                "model": {
                    "modelId": "flash",
                    "displayName": "flash",
                    "status": "active",
                    "capabilities": {
                        "modalities": ["text"], "toolCalling": true, "streaming": true,
                        "structuredOutput": true, "vision": false, "audio": false,
                        "reasoning": false, "contextWindow": 128000, "maxOutputTokens": 16384
                    }
                },
                "source": "default",
                "enabled": true,
                "endpoint": "openai-compatible",
                "actions": ["set_enabled", "edit"]
            }]
        }]});
        // The picker's load, then the panel's own read.
        for _ in 0..2 {
            let request = wire.read("provider.management");
            wire.reply(&request, area.clone());
        }
        // Ctrl+R reads, refreshes, then the read that reports the new
        // catalog fails.
        let request = wire.read("provider.management");
        wire.reply(&request, area);
        let request = wire.read("provider.refresh");
        wire.reply(&request, json!({}));
        let request = wire.read("provider.management");
        wire.send(json!({"jsonrpc":"2.0","id":request["id"],"error":{
            "code":-32000,"message":"management read failed"}}));
        let mut line = String::new();
        while wire.0.read_line(&mut line).is_ok_and(|read| read > 0) {
            line.clear();
        }
    });
    let mut terminal = Terminal::launch_tui(home.command());
    terminal.until("test / model");
    shown.send(()).unwrap();
    terminal.write(b"/models\r");
    terminal.until("Test / flash");
    terminal.raw(b"\x1b[B");
    terminal.raw(b"\r");
    terminal.until("Models · Test");
    terminal.raw(b"\x12");
    terminal.until("Refreshed the models.");
    terminal.until("management read failed");
    terminal.signal(libc::SIGKILL);
    terminal.wait();
    daemon.join().unwrap();
}
