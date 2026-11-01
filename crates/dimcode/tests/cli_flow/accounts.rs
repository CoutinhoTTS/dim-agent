//! The `/connect` sign-in follows the account descriptor that
//! `provider.management` reports.
use super::*;

/// A `provider.management` reply with the given providers.
fn management(providers: Value) -> Value {
    json!({"providers": providers, "errors": []})
}

/// One described provider, with the connection fields the panel reads.
fn described(id: &str, name: &str, namespace: &str, methods: &[&str], signed_in: bool) -> Value {
    json!({
        "providerId": id,
        "displayName": name,
        "source": "builtin",
        "version": 1,
        "enabled": true,
        "preferences": {},
        "connection": {
            "driverKind": "openai-codex",
            "auth": {"type": "bearer"},
            "baseUrl": "https://fixture.invalid/v1",
            "credentialConfigured": signed_in,
        },
        "account": {
            "namespace": namespace,
            "loginMethods": methods,
            "state": {"authenticated": signed_in},
        },
    })
}

fn hello(wire: &mut Wire, extra: &[&str]) {
    let hello = wire.read("hello");
    let mut features = vec!["session.create", "session.history", "provider.list"];
    features.extend(extra);
    wire.reply(&hello,json!({"instanceId":"server","serverVersion":env!("CARGO_PKG_VERSION"),"protocolVersion":3,"daemonProfile":{"kind":"product"},"features":features}));
}

/// The `/connect` panel signs in through each described namespace; a
/// daemon without the method shows the error in the sign-in step.
#[test]
fn the_provider_panel_signs_in_through_each_described_namespace() {
    for fullscreen in [false, true] {
        let home = Home::new();
        let daemon = home.daemon(|wire| {
            hello(
                wire,
                &[
                    "provider.management",
                    "provider.create",
                    "provider.update",
                    "provider.probe",
                    "account.codex.login.browser.begin",
                    "account.codex.login.cancel",
                    "account.grok.login.begin",
                    "account.grok.login.cancel",
                ],
            );
            wire.created();
            let attach = wire.read("session.attach");
            wire.reply(&attach, attach_record(&attach));
            let history = wire.read("session.history");
            wire.reply(
                &history,
                json!({"events":[],"through":8,"nextAfter":8,"done":true}),
            );
            wire.no_context();
            let providers = wire.read("account.providers");
            wire.reply(&providers, json!([]));
            let listed = wire.read("provider.management");
            wire.reply(
                &listed,
                management(json!([
                    described(
                        "openai-codex",
                        "OpenAI Codex",
                        "account.codex",
                        &["browser"],
                        false
                    ),
                    described("grok-build", "Grok Build", "account.grok", &["device"], false),
                ])),
            );
            let begin = wire.read("account.codex.login.browser.begin");
            assert_eq!(begin["params"], json!({}));
            wire.reply(&begin,json!({"attemptId":"codex-attempt","authorizationUrl":"urn:fixture:codex","expiresAt":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() + 60_000}));
            let cancel = wire.read("account.codex.login.cancel");
            assert_eq!(cancel["params"], json!({"attemptId":"codex-attempt"}));
            wire.reply(&cancel, json!({}));
            let begin = wire.read("account.grok.login.begin");
            assert_eq!(begin["params"], json!({}));
            wire.reply(&begin,json!({"attemptId":"grok-attempt","verificationUri":"https://fixture.invalid/device","userCode":"GROK-CODE","expiresAt":999999}));
            let cancel = wire.read("account.grok.login.cancel");
            assert_eq!(cancel["params"], json!({"attemptId":"grok-attempt"}));
            wire.reply(&cancel, json!({}));
            wire.hold();
        });
        let mut command = home.command();
        command.env("DIMCODE_OAUTH_MANUAL", "1");
        if fullscreen {
            command.arg("--fullscreen");
        }
        sized(&mut command, 100, 32);
        let mut terminal = Terminal::launch_tui(command);
        terminal.until("test / model");
        terminal.write(b"/connect\r");
        terminal.until("OpenAI Codex");
        terminal.write(b"\r");
        terminal.until("Browser login");
        terminal.until("urn:fixture:codex");
        terminal.write(b"\x1b");
        terminal.until("Grok Build");
        terminal.write(b"\x1b[B\r");
        terminal.until("GROK-CODE");
        terminal.write(b"\x1b");
        terminal.until("Add custom provider");
        terminal.write(b"\x1b");
        terminal.signal(libc::SIGKILL);
        terminal.wait();
        daemon.join().unwrap();
    }
}

#[test]
fn a_panel_without_the_sign_in_method_shows_the_unsupported_daemon_error() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        hello(
            wire,
            &[
                "provider.management",
                "provider.create",
                "provider.update",
                "provider.probe",
            ],
        );
        wire.created();
        let attach = wire.read("session.attach");
        wire.reply(&attach, attach_record(&attach));
        let history = wire.read("session.history");
        wire.reply(
            &history,
            json!({"events":[],"through":8,"nextAfter":8,"done":true}),
        );
        wire.no_context();
        let providers = wire.read("account.providers");
        wire.reply(&providers, json!([]));
        let listed = wire.read("provider.management");
        wire.reply(
            &listed,
            management(json!([described(
                "openai-codex",
                "OpenAI Codex",
                "account.codex",
                &["browser"],
                false
            )])),
        );
        wire.hold();
    });
    let mut command = home.command();
    command.arg("--fullscreen");
    sized(&mut command, 100, 32);
    let mut terminal = Terminal::launch_tui(command);
    terminal.until("test / model");
    terminal.write(b"/connect\r");
    terminal.until("OpenAI Codex");
    terminal.write(b"\r");
    terminal.until("This daemon does not offer account.codex.login.browser.begin");
    terminal.signal(libc::SIGKILL);
    terminal.wait();
    daemon.join().unwrap();
}
