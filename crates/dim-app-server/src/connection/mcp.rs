//! Local product wire projections over native MCP owners.
use super::*;
pub(super) use crate::mcp::management::error;
use crate::{
    agents::AgentHost,
    mcp::{McpSession, McpSessionState, McpSourceState},
};
use dim_agent_capabilities::tool_registry::{ToolSourcesState, ToolSourcesView};
use rutis::{EventKey, FiberState, FiberStatusChanged, Listener};
use tokio::sync::watch;

fn project_state(id: &str, state: McpSessionState, tools: ToolSourcesState) -> McpStateResult {
    McpStateResult {
        session_id: id.to_owned(),
        servers: state
            .servers
            .into_iter()
            .map(|(id, state)| {
                let status = tools.0.get(&format!("mcp:{id}"));
                let (state, version, message) = match state {
                    McpSourceState::Disabled => ("disabled", None, None),
                    McpSourceState::ApprovalRequired => ("approval_required", None, None),
                    McpSourceState::Connecting => ("connecting", None, None),
                    McpSourceState::Ready { version } => ("ready", Some(version), None),
                    McpSourceState::Failed { message } => ("failed", None, Some(message)),
                };
                McpServerStatus {
                    id,
                    state,
                    version,
                    message,
                    adopted_version: status.and_then(|s| s.adopted.clone()),
                    rejection: status
                        .and_then(|s| s.rejected.as_ref())
                        .map(|r| McpRejection {
                            version: r.version.clone(),
                            reason: r.reason.clone(),
                        }),
                }
            })
            .collect(),
        catalog_error: state.catalog_error,
        closed: state.closed,
    }
}
pub(super) fn session_state(session: &Session) -> Result<McpStateResult, RpcError> {
    Ok(project_state(
        session.id(),
        session.session_service::<McpSession>()?.state(),
        session.session_service::<ToolSourcesView>()?.get(),
    ))
}

/// Reacquire services when their Plugins become active; never poll readiness.
pub(super) struct Availability(pub(super) watch::Sender<()>);
impl Listener<FiberStatusChanged> for Availability {
    fn call<'a>(
        &'a self,
        _ctx: &'a Ctx,
        event: &'a FiberStatusChanged,
    ) -> BoxFuture<'a, Result<Option<()>, CordisError>> {
        if event.to == FiberState::Active {
            self.0.send_replace(());
        }
        Box::pin(async { Ok(None) })
    }
}
impl Connection {
    pub(super) async fn follow_mcp(
        &self,
        host: Arc<AgentHost>,
        session: Session,
        generation: u64,
        cancel: CancellationToken,
    ) {
        let (signal, mut active) = watch::channel(());
        let Ok(listener) = host
            .ctx
            .events()
            .on(&host.ctx, &EventKey::of(), Availability(signal))
        else {
            return;
        };
        let mut owners: Option<(Arc<McpSession>, Arc<ToolSourcesView>)> = None;
        let mut watches = None;
        let mut last = None;
        loop {
            if let (Ok(mcp), Ok(tools)) = (
                session.session_service::<McpSession>(),
                session.session_service::<ToolSourcesView>(),
            ) && !mcp.state().closed
                && owners.as_ref().is_none_or(|(old_mcp, old_tools)| {
                    !Arc::ptr_eq(old_mcp, &mcp) || !Arc::ptr_eq(old_tools, &tools)
                })
            {
                last = None;
                watches = Some((mcp.subscribe(), tools.subscribe()));
                owners = Some((mcp, tools));
            }
            if let Some((mcp, tools)) = &mut watches {
                let state = project_state(
                    session.id(),
                    mcp.borrow_and_update().clone(),
                    tools.borrow_and_update().clone(),
                );
                let closed = state.closed;
                if last.as_ref() != Some(&state) {
                    last = Some(state.clone());
                    if !self.notify(session.id(), generation, Notification::McpState(state)) {
                        break;
                    }
                }
                if closed {
                    watches = None;
                    owners = None;
                    continue;
                }
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = session.closed() => break,
                result = active.changed() => if result.is_err() { break },
                result = async {
                    match &mut watches {
                        Some((mcp, tools)) => tokio::select! { r = mcp.changed() => r, r = tools.changed() => r },
                        None => std::future::pending().await,
                    }
                } => if result.is_err() { watches = None; owners = None; continue; },
            }
        }
        let _ = listener.dispose().await;
    }
}
