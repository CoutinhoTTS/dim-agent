//! The client tree: the rutis root the `dimcode` client process owns
//! (docs/design/plugins/Tui.md). Its plugins hold the resources a client
//! invocation owns and release them in reverse order on every way out. The
//! daemon's own tree is a different process; the client reaches it only over
//! the protocol v3 connection `DaemonLink` provides.

use std::sync::Mutex;

use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};

use crate::args::Options;
use crate::client::{self, Client};

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
