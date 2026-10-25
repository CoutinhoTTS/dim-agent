//! The UI registry (Tui design §3.6): interfaces register the connection
//! they already have, any number per Session; a shutdown is a notification
//! on that connection; registrations leave with their connection's
//! attachment or the connection itself; the methods are local-only.
use super::*;

async fn attached_session(client: &mut Client) -> String {
    let created = client
        .rpc("session.create", json!({"cwd": std::env::temp_dir()}))
        .await;
    assert_ok(&created);
    let session_id = created["result"]["sessionId"].as_str().unwrap().to_owned();
    assert_ok(
        &client
            .rpc("session.attach", json!({"sessionId": session_id}))
            .await,
    );
    session_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interfaces_register_any_number_per_session_and_leave_on_request() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut first = Client::new_version(&server, false, 3).await;
    for method in [
        "ui.open",
        "ui.close",
        "ui.attach",
        "ui.status",
        "ui.shutdown",
        "ui.replace",
    ] {
        assert!(
            first.features.as_array().unwrap().contains(&json!(method)),
            "{method}"
        );
    }
    assert!(
        first
            .features
            .as_array()
            .unwrap()
            .contains(&json!("ui.leaf"))
    );
    let session_id = attached_session(&mut first).await;
    // A leaf that is not there is refused, and nothing is registered.
    let missing = first
        .rpc(
            "ui.open",
            json!({"sessionId": session_id, "kind": "tui", "peer": "ui-missing", "address": "unix:/nonexistent/leaf.sock"}),
        )
        .await;
    assert_eq!(
        missing["error"]["data"]["appError"], "ui_leaf_failed",
        "{missing}"
    );

    // Registering needs the attachment; then it is idempotent.
    let unattached = first
        .rpc("ui.attach", json!({"sessionId":"missing","kind":"tui"}))
        .await;
    assert_eq!(
        unattached["error"]["data"]["appError"],
        "session_not_attached"
    );
    let attached = first
        .rpc("ui.attach", json!({"sessionId": session_id, "kind": "tui"}))
        .await;
    assert_ok(&attached);
    let ui_id = attached["result"]["uiId"].as_str().unwrap().to_owned();
    let again = first
        .rpc("ui.attach", json!({"sessionId": session_id, "kind": "tui"}))
        .await;
    assert_eq!(again["result"]["uiId"], ui_id);

    // A second viewer of the same Session is just another registration.
    let mut second = Client::new_version(&server, false, 3).await;
    assert_ok(
        &second
            .rpc("session.attach", json!({"sessionId": session_id}))
            .await,
    );
    let other = second
        .rpc(
            "ui.attach",
            json!({"sessionId": session_id, "kind": "electron"}),
        )
        .await;
    assert_ok(&other);
    let status = second.rpc("ui.status", json!({})).await;
    let instances = status["result"]["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 2);
    assert!(
        instances
            .iter()
            .all(|i| i["sessionId"] == session_id && i["sinceMs"].is_u64())
    );
    assert_eq!(status["result"]["webConnections"], 0);

    // Asking the first to leave reaches the first connection only; the
    // Session is untouched and the second viewer stays.
    assert_ok(
        &second
            .rpc(
                "ui.shutdown",
                json!({"uiId": ui_id, "reason": "another terminal took over"}),
            )
            .await,
    );
    let notice = first.notice("ui.shutdown").await;
    assert_eq!(notice["params"]["uiId"], ui_id);
    assert_eq!(notice["params"]["reason"], "another terminal took over");
    drop(first);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = second.rpc("ui.status", json!({})).await;
            if status["result"]["instances"].as_array().unwrap().len() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the registration leaves with its connection");
    assert_eq!(
        second.rpc("ui.shutdown", json!({"uiId": ui_id})).await["error"]["data"]["appError"],
        "ui_unknown"
    );

    // Replacing: the caller registers and the old interface is asked to
    // leave; a vanished old interface does not undo the new registration.
    let mut third = Client::new_version(&server, false, 3).await;
    assert_ok(
        &third
            .rpc("session.attach", json!({"sessionId": session_id}))
            .await,
    );
    let old = other["result"]["uiId"].as_str().unwrap();
    let replaced = third
        .rpc(
            "ui.replace",
            json!({"uiId": old, "sessionId": session_id, "kind": "tui"}),
        )
        .await;
    assert_ok(&replaced);
    assert_eq!(
        second.notice("ui.shutdown").await["params"]["reason"],
        "replaced"
    );
    let gone = third
        .rpc(
            "ui.replace",
            json!({"uiId": "no-such-ui", "sessionId": session_id, "kind": "tui"}),
        )
        .await;
    assert_eq!(gone["error"]["data"]["appError"], "ui_unknown");
    let status = third.rpc("ui.status", json!({})).await;
    assert!(
        status["result"]["instances"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["uiId"] == replaced["result"]["uiId"])
    );

    // The registry is managed from local connections only.
    let mut bare = Client::new_version(&server, true, 3).await;
    assert_eq!(
        bare.rpc("ui.status", json!({})).await["error"]["data"]["appError"],
        "local_transport_required"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_registration_leaves_when_its_session_is_detached_or_closed() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let first = attached_session(&mut client).await;
    let second = attached_session(&mut client).await;
    for session_id in [&first, &second] {
        assert_ok(
            &client
                .rpc("ui.attach", json!({"sessionId": session_id, "kind": "tui"}))
                .await,
        );
    }
    let registered = |status: &Value| -> Vec<String> {
        status["result"]["instances"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["sessionId"].as_str().unwrap().to_owned())
            .collect()
    };

    // Detaching ends this connection's registration on that Session only.
    assert_ok(
        &client
            .rpc("session.detach", json!({"sessionId": first}))
            .await,
    );
    let status = client.rpc("ui.status", json!({})).await;
    assert_eq!(registered(&status), std::slice::from_ref(&second));

    // A closed Session takes its viewers' registrations with it.
    assert_ok(
        &client
            .rpc("session.close", json!({"sessionId": second}))
            .await,
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = client.rpc("ui.status", json!({})).await;
            if registered(&status).is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the registration leaves with the closed Session");
}

/// A leaf node as the interactive `dimcode` builds one (Tui design §3.6),
/// with a stand-in `Tui` that registers its protocol v3 connection and
/// counts its loads and unloads.
struct FakeLeaf {
    peer: String,
    address: String,
    root: rutis::Ctx,
    loaded: Arc<AtomicUsize>,
    unloaded: Arc<AtomicUsize>,
    directory: std::path::PathBuf,
}

impl FakeLeaf {
    async fn start(server: &RunningServer) -> Self {
        let (leaf, handle, features) = Self::unhosted(server).await;
        handle.set(features).await.unwrap();
        leaf
    }

    async fn unhosted(
        server: &RunningServer,
    ) -> (Self, rutis_bridge::PeerHandle, rutis_bridge::Features) {
        use rutis_bridge::{
            Described, Features, IdentityPlugin, LinkConfig, PeerPlugin, StaticCatalog,
            StaticIdentity, channel::PeerId,
        };
        let id = uuid::Uuid::new_v4().simple().to_string();
        let peer = format!("ui-{id}");
        let directory = std::env::temp_dir().join(format!("dim-leaf-{}", &id[..12]));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("leaf.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let loaded = Arc::new(AtomicUsize::new(0));
        let unloaded = Arc::new(AtomicUsize::new(0));
        let catalog = StaticCatalog::new().with(
            "Tui",
            Described {
                schema: None,
                version: None,
                integrity: None,
            },
            FakeTuiFactory {
                socket: server.unix_path.clone().unwrap(),
                loaded: loaded.clone(),
                unloaded: unloaded.clone(),
            },
        );
        let mut link = LinkConfig::dial(
            PeerId::new("dim-daemon").unwrap(),
            "ui-leaf",
            "ui-leaf",
            "accept",
        );
        link.reconnect = false;
        let root = rutis::Ctx::root().unwrap();
        root.plugin(LeafTransport(std::sync::Mutex::new(Some(listener))));
        root.plugin(IdentityPlugin::new(
            "ui-leaf",
            StaticIdentity::new(PeerId::new(peer.clone()).unwrap()),
        ));
        let features = Features {
            host: Some(Arc::new(catalog)),
            ..Features::default()
        };
        let node = PeerPlugin::new(link, Features::default()).unwrap();
        let handle = node.handle();
        let node = root.plugin(node);
        (&node).await.unwrap();
        (
            Self {
                peer,
                address: format!("unix:{}", path.display()),
                root,
                loaded,
                unloaded,
                directory,
            },
            handle,
            features,
        )
    }

    /// The process ends: its link closes.
    async fn exit(self) {
        let _ = self.root.shutdown().await;
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

struct LeafTransport(std::sync::Mutex<Option<std::os::unix::net::UnixListener>>);

impl rutis::Plugin for LeafTransport {
    fn name(&self) -> &str {
        "test/leaf-transport"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
    ) -> rutis::BoxFuture<'a, Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            let listener = self.0.lock().unwrap().take();
            ctx.provide_as::<dyn rutis_bridge::Transport>(
                rutis_bridge::transport_key("ui-leaf"),
                Arc::new(Accept(std::sync::Mutex::new(listener))),
            )?;
            Ok(rutis::Effect::Done)
        })
    }
}

struct Accept(std::sync::Mutex<Option<std::os::unix::net::UnixListener>>);

impl rutis_bridge::Transport for Accept {
    fn kind(&self) -> &str {
        "ui-leaf"
    }
    fn dial<'a>(
        &'a self,
        _: &'a rutis_bridge::Dial,
    ) -> rutis::BoxFuture<
        'a,
        Result<rutis_bridge::channel::Channel, rutis_bridge::channel::ConnectError>,
    > {
        Box::pin(async move {
            let listener = self.0.lock().unwrap().take().expect("one connection");
            listener.set_nonblocking(true).unwrap();
            let listener = tokio::net::UnixListener::from_std(listener).unwrap();
            let stream = listener.accept().await.unwrap().0.into_std().unwrap();
            stream.set_nonblocking(false).unwrap();
            let reader = stream.try_clone().unwrap();
            let closer = Arc::new(UnixCloser(stream.try_clone().unwrap()));
            Ok(rutis_bridge::transport::local::framed(
                reader, stream, closer,
            ))
        })
    }
}

struct UnixCloser(std::os::unix::net::UnixStream);

impl rutis_bridge::channel::Closer for UnixCloser {
    fn close(&self, _: &str) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

struct FakeTuiFactory {
    socket: std::path::PathBuf,
    loaded: Arc<AtomicUsize>,
    unloaded: Arc<AtomicUsize>,
}

impl rutis::PluginFactory<Value> for FakeTuiFactory {
    fn name(&self) -> &str {
        "Tui"
    }
    fn injects(&self) -> &[rutis::TypeKey] {
        &[]
    }
    fn build(&self, config: &Value) -> Result<Box<dyn rutis::Plugin>, rutis::CordisError> {
        Ok(Box::new(FakeTui {
            socket: self.socket.clone(),
            config: config.clone(),
            loaded: self.loaded.clone(),
            unloaded: self.unloaded.clone(),
        }))
    }
}

/// Registers as the leaf the way the hosted `Tui` does: its own protocol
/// v3 connection, `session.attach`, then `ui.attach` with the config's
/// identity. Unloading closes the connection.
struct FakeTui {
    socket: std::path::PathBuf,
    config: Value,
    loaded: Arc<AtomicUsize>,
    unloaded: Arc<AtomicUsize>,
}

impl rutis::Plugin for FakeTui {
    fn name(&self) -> &str {
        "Tui"
    }
    fn apply<'a>(
        &'a self,
        _: &'a rutis::Ctx,
    ) -> rutis::BoxFuture<'a, Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            let mut stream = BufReader::new(UnixStream::connect(&self.socket).await.unwrap());
            let session = self.config["sessionId"].clone();
            for (id, method, params) in [
                (
                    1,
                    "hello",
                    json!({"protocolVersion": 3, "clientKind": "cli"}),
                ),
                (2, "session.attach", json!({"sessionId": session})),
                (
                    3,
                    "ui.attach",
                    json!({"sessionId": session, "kind": "tui",
                        "uiId": self.config["uiId"], "token": self.config["token"]}),
                ),
            ] {
                let request =
                    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
                stream
                    .get_mut()
                    .write_all(format!("{request}\n").as_bytes())
                    .await
                    .unwrap();
                loop {
                    let mut line = String::new();
                    stream.read_line(&mut line).await.unwrap();
                    let reply: Value = serde_json::from_str(&line).unwrap();
                    if reply["id"] == id {
                        assert!(reply.get("error").is_none(), "{method}: {reply}");
                        break;
                    }
                }
            }
            self.loaded.fetch_add(1, Ordering::SeqCst);
            let unloaded = self.unloaded.clone();
            Ok(rutis::Effect::Disposer(Box::new(move || {
                drop(stream);
                unloaded.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })))
        })
    }
}

async fn states(client: &mut Client) -> Vec<(String, String)> {
    let status = client.rpc("ui.status", json!({})).await;
    status["result"]["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["uiId"].as_str().unwrap().to_owned(),
                i["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

async fn eventually(client: &mut Client, check: impl Fn(&[(String, String)]) -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if check(&states(client).await) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the registry reached the expected state");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_daemon_hosts_replaces_and_closes_leaf_nodes() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let session_id = attached_session(&mut client).await;
    let open = |leaf: &FakeLeaf, extra: Value| {
        let mut params = json!({"sessionId": session_id, "kind": "tui",
            "peer": leaf.peer, "address": leaf.address});
        for (key, value) in extra.as_object().into_iter().flatten() {
            params[key] = value.clone();
        }
        params
    };

    // Hosting: `ui.open` settles once the leaf's `Tui` is loaded; it is
    // online when that `Tui` attached.
    let (first, host, features) = FakeLeaf::unhosted(&server).await;
    let opened = {
        let mut state = host.link_state().unwrap();
        let opening = client.rpc("ui.open", open(&first, json!({})));
        tokio::pin!(opening);
        tokio::select! {
            result = &mut opening => panic!("ui.open ended before hosting was offered: {result}"),
            () = async {
                while !matches!(&*state.borrow_and_update(), rutis_bridge::LinkState::Ready { .. }) {
                    state.changed().await.unwrap();
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            } => {}
        }
        assert_eq!(first.loaded.load(Ordering::SeqCst), 0);
        host.set(features).await.unwrap();
        opening.await
    };
    assert_ok(&opened);
    assert_eq!(opened["result"]["uiId"], first.peer.as_str());
    assert_eq!(first.loaded.load(Ordering::SeqCst), 1);
    let first_id = first.peer.clone();
    eventually(&mut client, |s| {
        s == [(first_id.clone(), "online".to_owned())]
    })
    .await;

    // Configuration is checked at the boundary.
    let mut other = Client::new_version(&server, false, 3).await;
    assert_ok(
        &other
            .rpc("session.attach", json!({"sessionId": session_id}))
            .await,
    );
    let token = other
        .rpc(
            "ui.attach",
            json!({"sessionId": session_id, "kind": "tui", "uiId": first.peer, "token": "wrong"}),
        )
        .await;
    assert_eq!(token["error"]["data"]["appError"], "ui_token", "{token}");
    let generation_alone = client
        .rpc("ui.open", open(&first, json!({"expectedGeneration": 1})))
        .await;
    assert_eq!(
        generation_alone["error"]["code"], -32602,
        "{generation_alone}"
    );

    // Two replacements observing the same generation: exactly one wins and
    // the old leaf's `Tui` is unloaded.
    let generation = opened["result"]["generation"].clone();
    let (a, b) = (
        FakeLeaf::start(&server).await,
        FakeLeaf::start(&server).await,
    );
    let mut client_b = Client::new_version(&server, false, 3).await;
    assert_ok(
        &client_b
            .rpc("session.attach", json!({"sessionId": session_id}))
            .await,
    );
    let replace = json!({"replace": first.peer, "expectedGeneration": generation});
    let (ra, rb) = tokio::join!(
        other.rpc("ui.open", open(&a, replace.clone())),
        client_b.rpc("ui.open", open(&b, replace.clone())),
    );
    let (winner, loser, losing) = if ra.get("error").is_none() {
        (a, b, rb)
    } else {
        (b, a, ra)
    };
    assert_eq!(
        losing["error"]["data"]["appError"], "ui_generation",
        "{losing}"
    );
    assert_eq!(first.unloaded.load(Ordering::SeqCst), 1);
    assert_eq!(loser.loaded.load(Ordering::SeqCst), 0);
    let winner_id = winner.peer.clone();
    eventually(&mut client, |s| {
        s == [(winner_id.clone(), "online".to_owned())]
    })
    .await;

    // A failed replacement leaves the old leaf and releases its claim: a
    // later replacement still wins.
    let failed = client
        .rpc(
            "ui.open",
            json!({"sessionId": session_id, "kind": "tui", "peer": "ui-gone",
                "address": "unix:/nonexistent/leaf.sock", "replace": winner.peer}),
        )
        .await;
    assert_eq!(
        failed["error"]["data"]["appError"], "ui_leaf_failed",
        "{failed}"
    );
    assert_eq!(winner.unloaded.load(Ordering::SeqCst), 0);
    let third = FakeLeaf::start(&server).await;
    assert_ok(
        &client
            .rpc("ui.open", open(&third, json!({"replace": winner.peer})))
            .await,
    );
    assert_eq!(winner.unloaded.load(Ordering::SeqCst), 1);

    // A leaf whose process ends leaves the registry with its link.
    let third_id = third.peer.clone();
    eventually(&mut client, |s| {
        s == [(third_id.clone(), "online".to_owned())]
    })
    .await;
    third.exit().await;
    eventually(&mut client, |s| s.is_empty()).await;

    // `ui.close` unloads the hosted `Tui` and removes the registration.
    let fourth = FakeLeaf::start(&server).await;
    assert_ok(&client.rpc("ui.open", open(&fourth, json!({}))).await);
    assert_ok(&client.rpc("ui.close", json!({"uiId": fourth.peer})).await);
    assert_eq!(fourth.unloaded.load(Ordering::SeqCst), 1);
    assert!(states(&mut client).await.is_empty());

    for leaf in [first, winner, loser, fourth] {
        leaf.exit().await;
    }
}
