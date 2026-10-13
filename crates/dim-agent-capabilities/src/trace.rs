//! Optional Session capability. The host owns archive I/O and closes it after Agent shutdown.
pub mod archive;

use archive::TraceRecorder;
use dim_agent_runtime::{
    AgentBuilder, Capability, Diagnostics, InSession, KeysOf, SessionIdentity, SessionScope, scoped,
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};

/// The host archive and a host context that outlives every traced Session.
#[derive(Clone)]
pub struct TraceHost {
    pub ctx: Ctx,
    pub recorder: TraceRecorder,
}

/// Registers the host archive listener, then enables one Session's diagnostics.
pub struct TracePlugin {
    host: TraceHost,
    scope: SessionScope,
}
impl TracePlugin {
    pub fn new(scope: &SessionScope, host: TraceHost) -> Self {
        Self {
            host,
            scope: *scope,
        }
    }
}
impl TypedPlugin for TracePlugin {
    type Deps = (InSession<Diagnostics>, InSession<SessionIdentity>);
    fn name(&self) -> &str {
        "Trace"
    }
    fn apply<'a>(
        &'a self,
        _ctx: &'a Ctx,
        (InSession(diagnostics), InSession(identity)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            // The listener lives on the host context: closing records arrive
            // after this plugin unloads, and the host drains after them.
            self.host
                .recorder
                .listen(&self.host.ctx, &identity.session_id)?;
            if !diagnostics.enable() {
                return Err(CordisError::PluginFailed(
                    "Session trace already enabled".into(),
                ));
            }
            Ok(Effect::Done)
        })
    }
}
impl Capability for TracePlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

pub fn install(builder: &mut AgentBuilder, host: TraceHost) {
    builder.session_plugin(move |scope| TracePlugin::new(scope, host.clone()));
}
