//! MCP configuration directory. Connections belong to McpServer, not Settings.
pub(crate) mod management;
mod publication;
pub mod server;
mod session;
pub use publication::{McpApplication, McpOwner, McpPublication};
pub use session::{McpSession, McpSessionPlugin, McpSessionState, McpSourceState, install};

use crate::settings::{
    McpServerSettings, SettingsDocument, SettingsError, SettingsService, SettingsSource,
};
use dim_agent_runtime::{Capability, KeysOf};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypeKey, TypedPlugin};
use server::{McpServer, McpServerError};
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, Mutex},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error("MCP operations require a user or authorized project source")]
    InvalidSource,
    #[error("empty MCP server id")]
    InvalidId,
    #[error("MCP server is not configured in this source")]
    UnknownServer,
    #[error("MCP server is disabled")]
    Disabled,
    #[error("Project MCP execution requires approval of the current configuration")]
    ApprovalRequired,
    #[error("MCP server Plugin could not be loaded")]
    Unavailable,
    #[error(transparent)]
    Server(#[from] McpServerError),
    #[error("MCP directory is closed")]
    Closed,
    #[error("MCP operation cancelled")]
    Cancelled,
    #[error("MCP worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
}
pub type Result<T> = std::result::Result<T, McpError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum McpApproval {
    NotRequired,
    Required,
    Approved,
}

/// Server-side configuration, including secrets. Not a wire or logging type.
#[derive(Clone)]
pub struct McpServerEntry {
    pub source: SettingsSource,
    pub configuration: McpServerSettings,
    pub approval: McpApproval,
    pub approval_token: Option<String>,
}
impl McpServerEntry {
    pub fn executable(&self) -> bool {
        self.enabled() && self.approval != McpApproval::Required
    }

    pub fn enabled(&self) -> bool {
        match &self.configuration {
            McpServerSettings::Stdio { enabled, .. }
            | McpServerSettings::Remote { enabled, .. } => enabled.unwrap_or(true),
        }
    }
}

/// Source CAS tokens are distinct from connection and tool-source versions.
/// Sources are read independently, not in a cross-source transaction.
pub struct McpCatalog {
    pub user_revision: String,
    pub project_revision: Option<String>,
    pub servers: BTreeMap<String, McpServerEntry>,
}

pub enum McpChange {
    Put(McpServerSettings),
    Remove,
    SetEnabled(bool),
}

/// A lease on one connection generation. Tool handlers retain this lease,
/// not a lookup into the directory's newest generation.
pub struct McpConnection {
    service: Arc<McpServer>,
    released: tokio::sync::watch::Sender<()>,
    authorization: Option<(Arc<publication::Publication>, String)>,
}
impl McpConnection {
    async fn authorize(&self, cancel: &CancellationToken) -> server::Result<()> {
        if let Some((publication, reference)) = &self.authorization {
            publication
                .check(reference.clone(), cancel)
                .await
                .map_err(|error| match error {
                    McpError::Cancelled => McpServerError::Cancelled,
                    McpError::Closed => McpServerError::Closed,
                    _ => McpServerError::ApprovalRequired,
                })?;
        }
        Ok(())
    }

    pub fn state(&self) -> server::McpServerState {
        self.service.state()
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<server::McpServerState> {
        self.service.subscribe()
    }
    pub fn tool_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.service.tool_changes()
    }
    pub async fn tools(&self, cancel: &CancellationToken) -> server::Result<Vec<server::McpTool>> {
        self.authorize(cancel).await?;
        self.service.tools(cancel).await
    }
    pub async fn call(
        &self,
        name: String,
        arguments: dim_agent_types::JsonObject,
        cancel: &CancellationToken,
    ) -> server::Result<dim_agent_types::ToolResult> {
        self.authorize(cancel).await?;
        self.service.call(name, arguments, cancel).await
    }
}
impl Drop for McpConnection {
    fn drop(&mut self) {
        // Release only a binding. The host publishes retirement eligibility;
        // only the loader can remove a configured generation's plugin.
        self.released.send_replace(());
    }
}

pub struct McpService {
    settings: Arc<SettingsService>,
    publication: Arc<publication::Publication>,
    stopping: CancellationToken,
    gate: Mutex<()>,
    tasks: TaskTracker,
}
impl McpService {
    /// Acquire a generation in an explicit owner's loader subtree.
    pub async fn server(
        self: &Arc<Self>,
        instance_owner: McpOwner,
        project: Option<usize>,
        id: String,
        cancel: &CancellationToken,
    ) -> Result<Arc<McpConnection>> {
        let owner = self.clone();
        self.run(cancel, move |_, cancel| async move {
            owner
                .publication
                .acquire(instance_owner, project, id, &cancel)
                .await
        })
        .await
    }

    /// Explicitly close one source, including a source removed from settings.
    /// Old handles remain closed; a later server() creates a new generation.
    pub async fn disconnect(
        self: &Arc<Self>,
        source: SettingsSource,
        id: String,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if source == SettingsSource::Skills {
            return Err(McpError::InvalidSource);
        }
        let owner = self.clone();
        self.run(cancel, move |_, cancel| async move {
            owner.publication.disconnect(source, id, &cancel).await
        })
        .await
    }

    pub fn application(&self, source: SettingsSource) -> McpApplication {
        self.publication.application(source)
    }

    /// Only trusted host management may grant execution. The token identifies
    /// the exact effective project configuration the caller reviewed.
    pub async fn approve(
        self: &Arc<Self>,
        project: usize,
        id: String,
        token: String,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let owner = self.clone();
        self.run(cancel, move |_, cancel| async move {
            owner
                .publication
                .approval(
                    SettingsSource::ProjectMcp(project),
                    id,
                    Some(token),
                    &cancel,
                )
                .await
        })
        .await
    }

    pub async fn revoke(
        self: &Arc<Self>,
        project: usize,
        id: String,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let owner = self.clone();
        self.run(cancel, move |_, cancel| async move {
            owner
                .publication
                .approval(SettingsSource::ProjectMcp(project), id, None, &cancel)
                .await
        })
        .await
    }

    /// Project ids are host-authorized FileStorage indices, never client paths.
    /// Project entries shadow user entries even when disabled.
    pub async fn catalog(
        &self,
        project: Option<usize>,
        cancel: &CancellationToken,
    ) -> Result<McpCatalog> {
        // Explicit reads also request convergence of externally edited workspace
        // files. This is coalesced and is not an application acknowledgement.
        self.publication.refresh();
        self.run(cancel, move |settings, cancel| async move {
            let mut catalog = McpCatalog {
                user_revision: String::new(),
                project_revision: None,
                servers: BTreeMap::new(),
            };
            for source in std::iter::once(SettingsSource::UserMcp)
                .chain(project.map(SettingsSource::ProjectMcp))
            {
                let snapshot = settings.read(source, &cancel).await?;
                match source {
                    SettingsSource::UserMcp => catalog.user_revision = snapshot.revision,
                    SettingsSource::ProjectMcp(_) => {
                        catalog.project_revision = Some(snapshot.revision);
                    }
                    SettingsSource::Skills => unreachable!("only MCP sources are selected"),
                }
                let SettingsDocument::Mcp(document) = snapshot.document else {
                    unreachable!("MCP sources decode to MCP documents")
                };
                for (id, configuration) in document.mcp_servers {
                    let configuration = settings.mcp_effective(source, configuration)?;
                    let (approval, approval_token) =
                        if matches!(source, SettingsSource::ProjectMcp(_)) {
                            let token = settings.mcp_fingerprint(source, &id, &configuration)?;
                            let approved =
                                settings.mcp_approved(source, &id, &token, &cancel).await?;
                            (
                                if approved {
                                    McpApproval::Approved
                                } else {
                                    McpApproval::Required
                                },
                                Some(token),
                            )
                        } else {
                            (McpApproval::NotRequired, None)
                        };
                    catalog.servers.insert(
                        id,
                        McpServerEntry {
                            source,
                            configuration,
                            approval,
                            approval_token,
                        },
                    );
                }
            }
            Ok(catalog)
        })
        .await
    }

    /// Mutate exactly one persisted source, not the merged effective view.
    /// Concurrent edits fail CAS rather than overwriting another server.
    pub async fn update(
        &self,
        source: SettingsSource,
        expected: String,
        id: String,
        change: McpChange,
        cancel: &CancellationToken,
    ) -> Result<String> {
        if source == SettingsSource::Skills {
            return Err(McpError::InvalidSource);
        }
        if id.trim().is_empty() {
            return Err(McpError::InvalidId);
        }
        self.run(cancel, move |settings, cancel| async move {
            let snapshot = settings.read(source, &cancel).await?;
            if snapshot.revision != expected {
                return Err(SettingsError::Conflict.into());
            }
            let SettingsDocument::Mcp(mut document) = snapshot.document else {
                unreachable!("MCP sources decode to MCP documents")
            };
            match change {
                McpChange::Put(configuration) => {
                    document.mcp_servers.insert(id, configuration);
                }
                McpChange::Remove => {
                    document
                        .mcp_servers
                        .remove(&id)
                        .ok_or(McpError::UnknownServer)?;
                }
                McpChange::SetEnabled(value) => {
                    let configuration = document
                        .mcp_servers
                        .get_mut(&id)
                        .ok_or(McpError::UnknownServer)?;
                    let (McpServerSettings::Stdio { enabled, .. }
                    | McpServerSettings::Remote { enabled, .. }) = configuration;
                    *enabled = Some(value);
                }
            }
            Ok(settings
                .update(source, &expected, SettingsDocument::Mcp(document), &cancel)
                .await?
                .revision)
        })
        .await
    }

    // Admission and draining belong to this Plugin. Settings still owns the
    // file commit: cancellation cannot discard an already admitted commit.
    async fn run<T, F, Fut>(&self, cancel: &CancellationToken, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<SettingsService>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        let task = {
            let _guard = self.gate.lock().unwrap();
            if self.stopping.is_cancelled() {
                return Err(McpError::Closed);
            }
            if cancel.is_cancelled() {
                return Err(McpError::Cancelled);
            }
            let stopping = self.stopping.clone();
            let cancel = cancel.child_token();
            let settings = self.settings.clone();
            self.tasks.spawn(async move {
                let operation = operation(settings, cancel.clone());
                tokio::pin!(operation);
                tokio::select! {
                    biased;
                    _ = stopping.cancelled() => {},
                    result = &mut operation => return result,
                }
                cancel.cancel();
                operation.await
            })
        };
        task.await?
    }
}

pub struct McpPlugin;
impl TypedPlugin for McpPlugin {
    type Deps = (Arc<SettingsService>, Arc<publication::Publication>);
    fn name(&self) -> &str {
        "Mcp"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (settings, publication): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Arc::new(McpService {
                settings,
                publication,
                stopping: ctx.cancellation_token(),
                gate: Mutex::new(()),
                tasks: TaskTracker::new(),
            });
            let owner = service.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        {
                            let _guard = owner.gate.lock().unwrap();
                            owner.tasks.close();
                        }
                        owner.tasks.wait().await;
                        Ok(())
                    })
                }))
            })?;
            management::provide(ctx, service.clone(), service.settings.clone())?;
            ctx.provide_as(TypeKey::of::<McpService>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for McpPlugin {
    fn keys(&self) -> KeysOf<Self> {
        ((), ())
    }
}
