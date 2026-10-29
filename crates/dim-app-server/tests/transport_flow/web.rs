//! Web through actual C4, Settings, Permission and ToolExecutor boundaries.
use super::*;
use dim_app_server::managed_provider::{
    AccountSnapshot, CredentialLease, ManagedCatalog, ManagedError, ManagedFuture, ManagedProvider,
    ManagedProviderRow, ProviderDescriptor, WebSearchHit, WebSearchRequest, WebSearchResponse,
    management::{
        BrowserLogin, BrowserLoginBegin, DeviceLogin, LoginBegin, LoginState,
        ManagedBrowserLoginComplete, ManagedLoginAttempt, Usage,
    },
};
use rutis::host::BoxFuture;
use rutis::{Ctx, Plugin};
use std::sync::atomic::AtomicBool;
use tokio::io::AsyncReadExt;

struct SearchProvider(Arc<AtomicBool>);
fn unused<T: Send + 'static>() -> ManagedFuture<T> {
    Box::pin(async { Err(ManagedError::InvalidInput) })
}
impl ManagedProvider for SearchProvider {
    fn providers(&self) -> Vec<ProviderDescriptor> {
        vec![ProviderDescriptor {
            provider_id: "dimcode-api-oauth".into(),
            display_name: "Dim OAuth".into(),
            login_methods: vec!["device".into()],
        }]
    }
    fn status(&self, _: &str) -> ManagedFuture<AccountSnapshot> {
        unused()
    }
    fn recover(&self, _: &str) -> ManagedFuture<AccountSnapshot> {
        unused()
    }
    fn begin_browser(&self, _: BrowserLoginBegin) -> ManagedFuture<BrowserLogin> {
        unused()
    }
    fn complete_browser(&self, _: ManagedBrowserLoginComplete) -> ManagedFuture<AccountSnapshot> {
        unused()
    }
    fn begin_device(&self, _: LoginBegin) -> ManagedFuture<DeviceLogin> {
        unused()
    }
    fn attempt_status(&self, _: &ManagedLoginAttempt) -> Result<LoginState, ManagedError> {
        Err(ManagedError::AttemptNotFound)
    }
    fn cancel_attempt(&self, _: ManagedLoginAttempt) -> ManagedFuture<()> {
        unused()
    }
    fn logout(&self, _: &str) -> ManagedFuture<AccountSnapshot> {
        unused()
    }
    fn usage(&self, _: &str) -> ManagedFuture<Usage> {
        unused()
    }
    fn web_search(
        &self,
        req: WebSearchRequest,
        _: CancellationToken,
    ) -> ManagedFuture<WebSearchResponse> {
        let signed_in = self.0.load(Ordering::SeqCst);
        Box::pin(async move {
            if !signed_in {
                return Err(ManagedError::TokenMissing);
            }
            Ok(WebSearchResponse {
                results: vec![
                    WebSearchHit {
                        title: format!("<b>Managed {}</b>", req.query),
                        url: "https://managed.example.com/hit".into(),
                        snippet: "Managed <i>snippet</i>".into(),
                        published_at: Some("2026-10-28".into()),
                    },
                    WebSearchHit {
                        title: "Blocked".into(),
                        url: "https://sub.blocked.test/ignored".into(),
                        snippet: "ignored".into(),
                        published_at: None,
                    },
                ],
            })
        })
    }
    fn catalog(&self, _: &str) -> ManagedFuture<ManagedCatalog> {
        unused()
    }
    fn credential(&self, _: &str, _: &str) -> ManagedFuture<CredentialLease> {
        unused()
    }
    fn recover_rejected(&self, _: &CredentialLease) -> ManagedFuture<CredentialLease> {
        unused()
    }
    fn finish_rejected(&self, _: &CredentialLease) -> ManagedFuture<()> {
        unused()
    }
    fn quiesce(&self) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
}
struct SearchProviderPlugin(Arc<AtomicBool>);
impl Plugin for SearchProviderPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "search-provider-fixture"
    }
    async fn apply(&self, ctx: &Ctx, (): ()) -> rutis::Result {
        ctx.provide_as::<dyn ManagedProvider>(Arc::new(SearchProvider(self.0.clone())))?;
        Ok(())
    }
}

async fn configure(client: &mut Client, document: Value) {
    let read = client
        .rpc("settings.read", json!({"source":"web","scope":"user"}))
        .await;
    assert_ok(&read);
    assert_ok(
        &client
            .rpc(
                "settings.update",
                json!({"source":"web","scope":"user",
        "expectedRevision":read["result"]["revision"],"document":document}),
            )
            .await,
    );
}

async fn create_web(client: &mut Client, preset: &str, interactive: bool) -> String {
    let created = client
        .rpc(
            "session.create",
            json!({"cwd":std::env::temp_dir(),
        "tools":["web_search","web_fetch"],"interactive":interactive,
        "permissionPolicy":{"preset":preset,"allowedTools":[],"disallowedTools":[]}}),
        )
        .await;
    assert_ok(&created);
    created["result"]["sessionId"].as_str().unwrap().to_owned()
}

async fn result(session: &dim_agent_runtime::Session) -> ToolResult {
    session
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .rev()
        .find_map(|event| {
            if let EventKind::ToolResult(batch) = &event.kind {
                batch.results.first().map(|entry| entry.result.clone())
            } else {
                None
            }
        })
        .expect("tool result")
}

fn text(result: &ToolResult) -> &str {
    match &result.content {
        dim_agent_types::ToolContent::Text(text) => text,
        _ => panic!("Web returns text"),
    }
}

async fn submit(client: &mut Client, id: &str, name: &str, arguments: Value) {
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":id,"branchId":"main",
        "text":format!("tool:{name} {arguments}")}),
            )
            .await,
    );
}

async fn idle(client: &mut Client, id: &str) {
    assert_ok(
        &client
            .rpc("session.idle", json!({"sessionId":id,"branchId":"main"}))
            .await,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn web_settings_permission_cancellation_and_restore() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = requests.clone();
    let (slow, mut entered) = tokio::sync::mpsc::unbounded_channel();
    let (closed, mut disconnected) = tokio::sync::mpsc::unbounded_channel();
    let stop = CancellationToken::new();
    let stopping = stop.clone();
    let http = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = stopping.cancelled() => break,
                Some(done) = tasks.join_next() => { done.unwrap(); },
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted.unwrap();
                    let (count, slow, closed) = (request_count.clone(), slow.clone(), closed.clone());
                    tasks.spawn(async move {
                        let mut header = Vec::new();
                        let mut chunk = [0; 4096];
                        while !header.windows(4).any(|part| part == b"\r\n\r\n") {
                            let n = socket.read(&mut chunk).await.unwrap();
                            if n == 0 { return; }
                            header.extend_from_slice(&chunk[..n]);
                            assert!(header.len() < 16 * 1024);
                        }
                        let request = String::from_utf8(header).unwrap();
                        assert!(request.starts_with("GET "), "{request}");
                        count.fetch_add(1, Ordering::SeqCst);
                        if request.starts_with("GET /slow ") {
                            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\npending").await.unwrap();
                            slow.send(()).unwrap();
                            assert_eq!(socket.read(&mut chunk).await.unwrap(), 0);
                            closed.send(()).unwrap();
                            return;
                        }
                        if request.starts_with("GET /truncated-gzip ") {
                            let mut response = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Encoding: gzip\r\nConnection: close\r\n\r\n".to_vec();
                            response.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff, 1, 14, 0, 0xf1, 0xff]);
                            response.extend_from_slice(b"<p>partial</p>");
                            socket.write_all(&response).await.unwrap();
                            return;
                        }
                        let (media, body) = if request.starts_with("GET /search?") {
                            assert!(request.to_lowercase().contains("x-subscription-token: fixture-web-key\r\n"));
                            ("application/json", r#"{"type":"search","web":{"results":[{"title":"fixture","url":"https://example.com/","description":"fixture-web-key snippet"}]}}"#)
                        } else {
                            ("text/html", "<title>Page</title><main><h1>Web flow</h1><a href='/next'>next</a></main>")
                        };
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        socket.write_all(response.as_bytes()).await.unwrap();
                    });
                }
            }
        }
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    });
    let home = std::fs::canonicalize("/tmp")
        .unwrap()
        .join(format!("dw-{}", uuid::Uuid::new_v4()));
    let signed_in = Arc::new(AtomicBool::new(false));
    let signed_in_row = signed_in.clone();
    let config = ServerConfig {
        product_home: Some(home.clone()),
        target_home: Some(home.join("rust")),
        managed_provider: Some(ManagedProviderRow(Arc::new(move |builtins, _| {
            let flag = signed_in_row.clone();
            builtins
                .builtins
                .register::<Value, _, _>("SearchProviderFixture", move |_| {
                    Ok(SearchProviderPlugin(flag.clone()))
                });
            "SearchProviderFixture".to_owned()
        }))),
        ..Default::default()
    };
    let mut saved = None;
    for restoring in [false, true] {
        let (server, _model_requests, _) = super::session_typescript::start(config.clone()).await;
        let mut client = Client::new_version(&server, false, 3).await;
        if !restoring {
            assert_ok(&client.rpc("provider.create", json!({"providerId":"018ec73f-0000-7000-8000-000000000011","displayName":"Fixture","connection":{"driverKind":"openai-compatible","baseUrl":"http://localhost/v1","auth":{"type":"anonymous"}},"models":[{"modelId":"fixture","displayName":"fixture","status":"active",
                    "capabilities":dim_agent_types::ModelCapabilities::default()}],"enabled":true})).await);
            assert_ok(
                &client
                    .rpc(
                        "provider.select",
                        json!({"providerId":"018ec73f-0000-7000-8000-000000000011","modelId":"fixture"}),
                    )
                    .await,
            );
        }
        let id = if restoring {
            let id: String = saved.clone().unwrap();
            let before = requests.load(Ordering::SeqCst);
            assert_ok(&client.rpc("session.attach", json!({"sessionId":id})).await);
            assert_eq!(
                requests.load(Ordering::SeqCst),
                before,
                "restore must not repeat network work"
            );
            id
        } else {
            let id = create_web(&mut client, "full-access", false).await;
            let session = server.agent().unwrap().get_session(&id).unwrap();
            submit(&mut client, &id, "web_search", json!({"query":"initial"})).await;
            idle(&mut client, &id).await;
            assert!(text(&result(&session).await).contains("search_unavailable"));
            signed_in.store(true, Ordering::SeqCst);
            submit(
                &mut client,
                &id,
                "web_search",
                json!({"query":"oauth-default","blocked_domains":["blocked.test"]}),
            )
            .await;
            idle(&mut client, &id).await;
            let managed_hit = result(&session).await;
            assert_ne!(managed_hit.is_error, Some(true), "{managed_hit:?}");
            assert!(text(&managed_hit).contains("Managed oauth-default -site:blocked.test"));
            assert!(text(&managed_hit).contains("https://managed.example.com/hit"));
            assert!(!text(&managed_hit).contains("sub.blocked.test"));
            assert_eq!(
                managed_hit.structured_content.as_ref().unwrap()["results"],
                json!([{
                    "title": "Managed oauth-default -site:blocked.test",
                    "url": "https://managed.example.com/hit",
                    "snippet": "Managed snippet",
                    "published_at": "2026-10-28"
                }])
            );
            submit(
                &mut client,
                &id,
                "web_search",
                json!({"query":"q".repeat(500),"blocked_domains":["blocked.test"]}),
            )
            .await;
            idle(&mut client, &id).await;
            let full_query = result(&session).await;
            assert_ne!(full_query.is_error, Some(true), "{full_query:?}");
            assert!(
                !text(&full_query).contains("-site:"),
                "filters that overflow the relay query limit apply locally only"
            );
            let before = requests.load(Ordering::SeqCst);
            submit(
                &mut client,
                &id,
                "web_fetch",
                json!({"url":"http://198.18.0.1/"}),
            )
            .await;
            idle(&mut client, &id).await;
            let fake_ip = result(&session).await;
            assert_eq!(fake_ip.is_error, Some(true));
            assert!(text(&fake_ip).contains("blocked_address: Reserved benchmark addresses"));
            assert_eq!(requests.load(Ordering::SeqCst), before);
            configure(&mut client, json!({"allowPrivateHosts":["127.0.0.1"],
                "searchService":{"kind":"brave","endpoint":format!("{base}/search"),"apiKey":"fixture-web-key"}})).await;
            submit(
                &mut client,
                &id,
                "web_search",
                json!({"query":"configured"}),
            )
            .await;
            idle(&mut client, &id).await;
            let searched = result(&session).await;
            assert_ne!(searched.is_error, Some(true), "{searched:?}");
            assert!(text(&searched).contains("https://example.com/"));
            assert!(
                !serde_json::to_string(&searched)
                    .unwrap()
                    .contains("fixture-web-key")
            );
            signed_in.store(false, Ordering::SeqCst);
            configure(&mut client, json!({"allowPrivateHosts":["127.0.0.1"]})).await;
            let before = requests.load(Ordering::SeqCst);
            submit(&mut client, &id, "web_search", json!({"query":"removed"})).await;
            idle(&mut client, &id).await;
            assert!(text(&result(&session).await).contains("search_unavailable"));
            assert_eq!(requests.load(Ordering::SeqCst), before);

            let denied = create_web(&mut client, "read-only", true).await;
            let before = requests.load(Ordering::SeqCst);
            submit(&mut client, &denied, "web_fetch", json!({"url":base})).await;
            idle(&mut client, &denied).await;
            assert_eq!(
                result(&server.agent().unwrap().get_session(&denied).unwrap())
                    .await
                    .is_error,
                Some(true)
            );
            assert_eq!(requests.load(Ordering::SeqCst), before);

            let asking = create_web(&mut client, "workspace-write", true).await;
            assert_ok(
                &client
                    .rpc("session.attach", json!({"sessionId":asking}))
                    .await,
            );
            submit(&mut client, &asking, "web_fetch", json!({"url":base})).await;
            let request = client.notice("approval.request").await;
            assert_eq!(
                requests.load(Ordering::SeqCst),
                before,
                "approval precedes I/O"
            );
            assert_ok(
                &client
                    .rpc(
                        "approval.respond",
                        json!({"sessionId":asking,"branchId":"main",
                "requestId":request["params"]["requestId"],"decision":"allow"}),
                    )
                    .await,
            );
            idle(&mut client, &asking).await;
            assert!(
                text(&result(&server.agent().unwrap().get_session(&asking).unwrap()).await)
                    .contains("# Web flow")
            );
            let cancelling = create_web(&mut client, "workspace-write", true).await;
            assert_ok(
                &client
                    .rpc("session.attach", json!({"sessionId":cancelling}))
                    .await,
            );
            let before = requests.load(Ordering::SeqCst);
            submit(&mut client, &cancelling, "web_fetch", json!({"url":base})).await;
            client.notice("approval.request").await;
            assert_ok(
                &client
                    .rpc(
                        "session.stop",
                        json!({"sessionId":cancelling,"branchId":"main"}),
                    )
                    .await,
            );
            idle(&mut client, &cancelling).await;
            assert_eq!(
                requests.load(Ordering::SeqCst),
                before,
                "cancelled approval must not start I/O"
            );
            let cancelled = server.agent().unwrap().get_session(&cancelling).unwrap();
            assert_eq!(
                cancelled
                    .local("main", ..)
                    .await
                    .unwrap()
                    .iter()
                    .filter(|event| matches!(event.kind, EventKind::ToolResult(_)))
                    .count(),
                1
            );
            id
        };
        let session = server.agent().unwrap().get_session(&id).unwrap();
        let before = requests.load(Ordering::SeqCst);
        submit(&mut client, &id, "web_fetch", json!({"url":base})).await;
        idle(&mut client, &id).await;
        let fetched = result(&session).await;
        assert!(
            text(&fetched).contains("# Web flow"),
            "restoring={restoring}: {fetched:?}"
        );
        assert_eq!(
            requests.load(Ordering::SeqCst),
            before + 1,
            "restored cache starts empty"
        );
        submit(
            &mut client,
            &id,
            "web_fetch",
            json!({"url":base,"start_char":1000}),
        )
        .await;
        idle(&mut client, &id).await;
        assert_eq!(
            requests.load(Ordering::SeqCst),
            before + 1,
            "pagination uses the Session cache"
        );
        submit(
            &mut client,
            &id,
            "web_fetch",
            json!({"url":format!("{base}/truncated-gzip")}),
        )
        .await;
        idle(&mut client, &id).await;
        let truncated = result(&session).await;
        assert_eq!(truncated.is_error, Some(true), "{truncated:?}");
        assert!(
            text(&truncated).contains("connect_failed"),
            "a gzip stream without its trailer is not a complete page: {truncated:?}"
        );
        let results_before = session
            .local("main", ..)
            .await
            .unwrap()
            .iter()
            .filter(|event| matches!(event.kind, EventKind::ToolResult(_)))
            .count();
        submit(
            &mut client,
            &id,
            "web_fetch",
            json!({"url":format!("{base}/slow")}),
        )
        .await;
        tokio::time::timeout(Duration::from_secs(5), entered.recv())
            .await
            .unwrap()
            .unwrap();
        assert_ok(
            &client
                .rpc("session.stop", json!({"sessionId":id,"branchId":"main"}))
                .await,
        );
        tokio::time::timeout(Duration::from_secs(5), disconnected.recv())
            .await
            .unwrap()
            .unwrap();
        idle(&mut client, &id).await;
        let results_after = session
            .local("main", ..)
            .await
            .unwrap()
            .iter()
            .filter(|event| matches!(event.kind, EventKind::ToolResult(_)))
            .count();
        assert_eq!(
            results_after,
            results_before + 1,
            "one terminal tool result on interrupt"
        );
        saved = Some(id);
        drop(client);
        server.shutdown();
        server.join().await.unwrap();
    }
    stop.cancel();
    http.await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}
