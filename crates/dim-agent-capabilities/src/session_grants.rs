//! The Session's file grants, read when Permission is about to ask.
use crate::host::HostFuture;
use dim_agent_types::grant::SessionGrant;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub trait ReadSessionGrants: Send + Sync + 'static {
    fn grants(&self, cancel: CancellationToken) -> HostFuture<'_, Vec<SessionGrant>>;
}

pub struct SessionGrantsHost(pub Arc<dyn ReadSessionGrants>);

pub struct NoSessionGrants;

impl ReadSessionGrants for NoSessionGrants {
    fn grants(&self, _: CancellationToken) -> HostFuture<'_, Vec<SessionGrant>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}
