//! The client tree: the rutis root the `dimcode` client process owns
//! (docs/design/plugins/Tui.md). Its plugins hold the resources a client
//! invocation owns and release them in reverse order on every way out. The
//! daemon's own tree is a different process; the client reaches it only over
//! the protocol v3 connection `DaemonLink` provides.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use tokio::sync::oneshot;

use crate::args::Options;
use crate::client::{self, Client};
use crate::session_host::SessionHost;
use crate::ui::{self, tui::Leave};

/// The protocol v3 connection after `hello`, provided by [`DaemonLink`]. One
/// owner at a time drives it: `take` hands it over. `None` when the link was
/// made with `start: false` and no daemon runs, or once it was taken.
pub struct Connection(Mutex<Option<Client>>);

impl Connection {
    pub fn take(&self) -> Option<Client> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

/// What a plugin that needs the connection says when it is gone.
const CONNECTION_TAKEN: &str = "App Server connection is not available";

/// Finds the daemon's endpoint under the product home, starts the daemon
/// when `start` and none answers, and says hello. The connection closes
/// with the service: dropping the last `Client` ends it.
pub struct DaemonLink {
    pub options: Options,
    /// Start a daemon when none runs; `daemon stop` only looks for one.
    pub start: bool,
}

impl TypedPlugin for DaemonLink {
    type Deps = ();

    fn name(&self) -> &str {
        "DaemonLink"
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let client = client::connect_inner(&self.options, self.start)
                .await
                .map_err(CordisError::PluginFailed)?;
            ctx.provide(Connection(Mutex::new(client)))?;
            Ok(Effect::Done)
        })
    }
}

/// The terminal interface: the session thread (`SessionHost`, on the
/// connection) and the render thread (`ui::tui::run`). How the interface
/// ended goes to the composition root through `leave`.
pub struct Tui {
    pub env: ui::Env,
    pub options: Options,
    /// Taken by the load; the render thread sends the result once.
    pub leave: Mutex<Option<oneshot::Sender<Leave>>>,
}

impl TypedPlugin for Tui {
    type Deps = (Arc<Connection>,);

    fn name(&self) -> &str {
        "Tui"
    }

    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (connection,): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            // Registered before the render thread enters raw mode, so every
            // way out of the tree hands the terminal back.
            ctx.effect(|| {
                Effect::Disposer(Box::new(|| {
                    ui::terminal::restore_if_held();
                    Ok(())
                }))
            })?;
            let client = connection
                .take()
                .ok_or_else(|| CordisError::PluginFailed(CONNECTION_TAKEN.into()))?;
            let leave = self
                .leave
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .ok_or_else(|| CordisError::PluginFailed("Tui was loaded twice".into()))?;
            let host = SessionHost::start(self.options.clone(), client);
            let env = self.env.clone();
            let render = std::thread::Builder::new()
                .name("dim-tui".into())
                .spawn(move || {
                    // The interface shuts the session thread down before it
                    // returns; a dropped sender means the root stopped waiting.
                    let _ = leave.send(ui::tui::run(Box::new(host), env));
                })
                .map_err(|error| CordisError::PluginFailed(error.into()))?;
            ctx.effect(|| {
                Effect::Disposer(Box::new(move || {
                    // Finished once `leave` was sent; bounded for an unload
                    // that did not wait for it.
                    let deadline = std::time::Instant::now() + Duration::from_secs(1);
                    while !render.is_finished() && std::time::Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    if render.is_finished() {
                        let _ = render.join();
                    }
                    Ok(())
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}
