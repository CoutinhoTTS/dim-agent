//! `dimcode ui-leaf`: the process of a UI leaf (Tui design §3.6). It is a
//! rutis node linked to the daemon that started it, offering one plugin in
//! its host catalog, `Tui`; the daemon loads and unloads it there. When the
//! user quits, the hosted `Tui` reports its exit code to the daemon and this
//! process disposes its tree (the terminal is restored) and exits with it.
//! When the link stops, the tree is disposed the same way.

use rutis::{BoxFuture, CordisError, Ctx, Effect, Plugin, PluginFactory, TypeKey};
use rutis_bridge::{
    Described, Dial, Features, IdentityPlugin, LinkConfig, LinkState, PeerPlugin, StaticCatalog,
    StaticIdentity, Transport,
    channel::{Channel, Closer, ConnectError, PeerId},
    transport::local::framed,
    transport_key,
};
use serde_json::Value as Json;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

const DAEMON_PEER: &str = "dim-daemon";
/// The identity key of the leaf link, as on the daemon's side.
const LINKS: &str = "ui-leaf";

/// Entry point for `dimcode ui-leaf --leaf-peer ID <channel>`; the local
/// transport appends the channel address.
pub fn run(args: impl IntoIterator<Item = String>) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Error: {error}");
            return 1;
        }
    };
    runtime.block_on(async {
        match run_inner(args).await {
            Ok(code) => code,
            Err(error) => {
                eprintln!("Error: {error}");
                1
            }
        }
    })
}

async fn run_inner(args: impl IntoIterator<Item = String>) -> Result<i32, String> {
    let (leaf_peer, channel) = parse_args(args)?;
    let daemon = PeerId::new(DAEMON_PEER).expect("valid daemon peer id");
    let (left, leaving) = oneshot::channel();
    let catalog = StaticCatalog::new().with(
        "Tui",
        Described {
            schema: Some(dim_cli::leaf::schema()),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            integrity: None,
        },
        TuiFactory {
            left: Arc::new(Mutex::new(Some(left))),
        },
    );
    let mut link = LinkConfig::dial(daemon, "local", LINKS, &channel);
    // The daemon owns this process: no reconnect once its link ends.
    link.reconnect = false;
    let peer = PeerPlugin::new(
        link,
        Features {
            host: Some(Arc::new(catalog)),
            ..Features::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let handle = peer.handle();
    let root = Ctx::root().map_err(|error| error.to_string())?;
    root.plugin(LeafTransportPlugin);
    root.plugin(IdentityPlugin::new(LINKS, StaticIdentity::new(leaf_peer)));
    let node = root.plugin(peer);
    (&node)
        .await
        .map_err(|error| format!("the leaf node did not start: {error}"))?;
    let mut state = handle.link_state().ok_or("the leaf link did not start")?;
    let stopped = async {
        loop {
            if matches!(&*state.borrow_and_update(), LinkState::Stopped { .. }) {
                return;
            }
            if state.changed().await.is_err() {
                return;
            }
        }
    };
    let code = tokio::select! {
        code = leaving => code.unwrap_or(1),
        () = stopped => 0,
    };
    // Unloads the hosted `Tui`, whose effect restores the terminal.
    let _ = root.shutdown().await;
    Ok(code)
}

/// Builds the hosted `Tui` from the daemon's config; its exit code ends
/// this process.
struct TuiFactory {
    left: Arc<Mutex<Option<oneshot::Sender<i32>>>>,
}

impl PluginFactory<Json> for TuiFactory {
    fn name(&self) -> &str {
        "Tui"
    }

    fn injects(&self) -> &[TypeKey] {
        &[]
    }

    fn build(&self, config: &Json) -> Result<Box<dyn Plugin>, CordisError> {
        let left = self.left.clone();
        dim_cli::leaf::hosted_tui(
            config,
            Box::new(move |code| {
                if let Some(left) = left.lock().unwrap_or_else(|p| p.into_inner()).take() {
                    let _ = left.send(code);
                }
            }),
        )
    }
}

/// The leaf's side of the local transport. The daemon starts this process
/// with a Unix socket path (Unix) or a loopback address plus the one-time
/// `RUTIS_CHANNEL_TOKEN` (Windows); this transport accepts both.
struct LeafTransportPlugin;

impl Plugin for LeafTransportPlugin {
    fn name(&self) -> &str {
        "ui-leaf/transport"
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as::<dyn Transport>(transport_key("local"), Arc::new(LeafTransport))?;
            Ok(Effect::Done)
        })
    }
}

struct LeafTransport;

impl Transport for LeafTransport {
    fn kind(&self) -> &str {
        "local"
    }

    fn dial<'a>(&'a self, dial: &'a Dial) -> BoxFuture<'a, Result<Channel, ConnectError>> {
        Box::pin(async move {
            let address = dial.address.as_str();
            if let Some(address) = address.strip_prefix("tcp:") {
                let stream = tokio::net::TcpStream::connect(address)
                    .await
                    .map_err(retryable)?;
                stream.set_nodelay(true).map_err(retryable)?;
                let stream = stream.into_std().map_err(retryable)?;
                stream.set_nonblocking(false).map_err(retryable)?;
                if let Ok(token) = std::env::var("RUTIS_CHANNEL_TOKEN") {
                    let mut writer = stream.try_clone().map_err(retryable)?;
                    use std::io::Write;
                    writeln!(writer, "{token}").map_err(retryable)?;
                    writer.flush().map_err(retryable)?;
                }
                let reader = stream.try_clone().map_err(retryable)?;
                let closer = Arc::new(TcpCloser(stream.try_clone().map_err(retryable)?));
                return Ok(framed(reader, stream, closer));
            }
            #[cfg(unix)]
            {
                let path = address.strip_prefix("unix:").unwrap_or(address);
                let stream = tokio::net::UnixStream::connect(path)
                    .await
                    .map_err(retryable)?;
                let stream = stream.into_std().map_err(retryable)?;
                stream.set_nonblocking(false).map_err(retryable)?;
                let reader = stream.try_clone().map_err(retryable)?;
                let closer = Arc::new(UnixCloser(stream.try_clone().map_err(retryable)?));
                Ok(framed(reader, stream, closer))
            }
            #[cfg(not(unix))]
            {
                let _ = address;
                Err(ConnectError::Incompatible {
                    reason: format!("the leaf transport cannot dial {address}"),
                })
            }
        })
    }
}

fn retryable(error: std::io::Error) -> ConnectError {
    ConnectError::Retryable {
        reason: error.to_string(),
    }
}

struct TcpCloser(std::net::TcpStream);

impl Closer for TcpCloser {
    fn close(&self, _reason: &str) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(unix)]
struct UnixCloser(std::os::unix::net::UnixStream);

#[cfg(unix)]
impl Closer for UnixCloser {
    fn close(&self, _reason: &str) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<(PeerId, String), String> {
    let mut leaf = None;
    let mut channel = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--leaf-peer" => leaf = Some(args.next().ok_or("--leaf-peer needs a value")?),
            value if value.starts_with('-') => {
                return Err(format!("unknown ui-leaf argument {value}"));
            }
            value => {
                if channel.replace(value.to_owned()).is_some() {
                    return Err("ui-leaf takes one channel address".into());
                }
            }
        }
    }
    let leaf = leaf.ok_or("--leaf-peer is missing")?;
    let channel = channel.ok_or("the leaf channel address is missing")?;
    Ok((
        PeerId::new(leaf).map_err(|error| error.to_string())?,
        channel,
    ))
}
