use super::*;

fn rows(start: usize, end: usize) -> Value {
    (start..end).map(|index|json!({"sessionId":format!("session-{index}"),"cwd":"/workspace","preview":format!("topic{index:04}"),"model":{"providerId":"test","modelId":"model"},"updatedAt":10000-index})).collect()
}

#[test]
fn tui_resume_catalog_pages_searches_and_closes_without_waiting_for_a_read() {
    let home = Home::new();
    let (ready, waiting) = std::sync::mpsc::channel();
    let daemon=home.daemon(move |wire| {
        wire.hello(&["session.catalog"]);
        wire.created();
        let attach=wire.read("session.attach");
        wire.reply(&attach,attach_record(&attach));
        let history=wire.read("session.history");
        wire.reply(&history,json!({"events":[],"through":8,"nextAfter":8,"done":true}));
        let providers=wire.read("account.providers");
        wire.reply(&providers,json!([]));
        let first=wire.read("session.catalog");
        assert_eq!(first["params"],json!({"limit":50,"query":"","withPreview":true}));
        wire.reply(&first,json!({"sessions":rows(0,50),"nextCursor":"second-page"}));
        let second=wire.read("session.catalog");
        assert_eq!(second["params"],json!({"limit":50,"query":"","withPreview":true,"cursor":"second-page"}));
        wire.reply(&second,json!({"sessions":rows(50,100)}));
        let query=wire.read("session.catalog");
        assert_eq!(query["params"],json!({"limit":50,"query":"ä title","withPreview":true}));
        wire.reply(&query,json!({"sessions":[{"sessionId":"off-page","cwd":"/workspace","title":"off-page result","preview":"different preview","model":{"providerId":"test","modelId":"model"},"updatedAt":2}]}));
        let cleared=wire.read("session.catalog");
        assert_eq!(cleared["params"]["query"],"");
        wire.reply(&cleared,json!({"sessions":rows(0,50)}));
        let held=wire.read("session.catalog");
        ready.send(()).unwrap();
        let usage=wire.read("session.context.usage");
        wire.send(json!({"jsonrpc":"2.0","id":usage["id"],"error":{"code":-32601,"message":"unsupported","data":{"appError":"unknown_method"}}}));
        wire.reply(&held,json!({"sessions":[{"sessionId":"obsolete","cwd":"/workspace","preview":"STALE CATALOG ROW","model":{"providerId":"test","modelId":"model"},"updatedAt":1}]}));
        let reopened=wire.read("session.catalog");
        wire.reply(&reopened,json!({"sessions":[{"sessionId":"fresh","cwd":"/workspace","preview":"fresh opening","model":{"providerId":"test","modelId":"model"},"updatedAt":3}]}));
        no_catalog(wire);
    });
    let mut terminal = Terminal::launch_tui(home.command());
    terminal.until("test / model");
    terminal.write(b"/resume\r");
    terminal.until("topic0000");
    terminal.write(b"\x1b[F");
    terminal.until("topic0049");
    terminal.write("ä title".as_bytes());
    terminal.until("off-page result");
    terminal.write(b"\x1b[27u");
    terminal.until("topic0000");
    terminal.write(b"\x12");
    waiting.recv_timeout(PATIENCE).unwrap();
    terminal.write(b"\x1b[27u");
    terminal.write(b"/usage\r");
    terminal.until("This daemon does not track context usage");
    terminal.write(b"/resume\r");
    terminal.until("fresh opening");
    assert!(!terminal.transcript.contains("STALE CATALOG ROW"));
    terminal.write(b"\x1b[27u");
    terminal.write(b"/exit\r");
    terminal.until("Bye!");
    assert!(terminal.wait().success());
    daemon.join().unwrap();
}
