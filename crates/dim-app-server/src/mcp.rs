//! MCP configuration directory. Connections belong to McpServer, not Settings.
pub mod server;
mod session;
pub use session::{McpSession, McpSessionPlugin, McpSessionState, McpSourceState, install};

use crate::settings::{
    McpServerSettings, SettingsDocument, SettingsError, SettingsService, SettingsSource,
};
use dim_agent_runtime::{Capability, KeysOf, capability_plugin, wait_active};
use rutis::{BoxFuture, CordisError, Ctx, Effect, FiberView, TypeKey, TypedPlugin};
use server::{McpServer, McpServerError, McpServerPlugin};
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, Mutex, Weak},
    time::Duration,
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

/// Server-side configuration, including secrets. Not a wire or logging type.
#[derive(Clone)]
pub struct McpServerEntry {
    pub source: SettingsSource,
    pub configuration: McpServerSettings,
}
impl McpServerEntry {
    pub fn enabled(&self) -> bool {
        match &self.configuration {
            McpServerSettings::Stdio { enabled, .. }
            | McpServerSettings::Remote { enabled, .. } => enabled.unwrap_or(true),
        }
    }
}

/// Each revision is a file CAS token, not a connection or tool-source version.
/// Files are read independently; this is not a cross-file transaction.
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
    view: FiberView,
    service: Arc<McpServer>,
}
impl McpConnection {
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
        self.service.tools(cancel).await
    }
    pub async fn call(
        &self,
        name: String,
        arguments: dim_agent_types::JsonObject,
        cancel: &CancellationToken,
    ) -> server::Result<dim_agent_types::ToolResult> {
        self.service.call(name, arguments, cancel).await
    }
}
impl Drop for McpConnection {
    fn drop(&mut self) {
        // rutis initiates shutdown synchronously; dropping the waiter does not
        // abandon cleanup. The parent still owns and drains the child subtree.
        drop(self.view.shutdown());
    }
}

#[derive(Default)]
struct ServerRegistration {
    current: Weak<McpConnection>,
    generations: Vec<Weak<McpConnection>>,
}

pub struct McpService {
    settings: Arc<SettingsService>,
    context: Ctx,
    servers: tokio::sync::Mutex<BTreeMap<(Option<usize>, String), ServerRegistration>>,
    stopping: CancellationToken,
    gate: Mutex<()>,
    tasks: TaskTracker,
}
impl McpService {
    /// Acquire a connection owner for the effective source. Initialization is
    /// lazy, outside Plugin apply; user sources are shared across projects.
    pub async fn server(
        self: &Arc<Self>,
        project: Option<usize>,
        id: String,
        cancel: &CancellationToken,
    ) -> Result<Arc<McpConnection>> {
        let owner = self.clone();
        self.run(cancel, move |_, cancel| async move {
            let mut servers = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(McpError::Cancelled),
                servers = owner.servers.lock() => servers,
            };
            servers.retain(|_, registration| {
                registration
                    .generations
                    .retain(|lease| lease.strong_count() > 0);
                !registration.generations.is_empty()
            });
            // Read after admission to the directory lock: a queued acquisition
            // must not replace a newer connection with an older file snapshot.
            let mut entry = owner
                .catalog(project, &cancel)
                .await?
                .servers
                .remove(&id)
                .ok_or(McpError::UnknownServer)?;
            let source = match entry.source {
                SettingsSource::UserMcp => None,
                SettingsSource::ProjectMcp(root) => Some(root),
                SettingsSource::Skills => unreachable!("MCP directory contains only MCP sources"),
            };
            let identity = (source, id);
            if !entry.enabled() {
                return Err(McpError::Disabled);
            }
            if let (Some(root), McpServerSettings::Stdio { cwd, .. }) =
                (source, &mut entry.configuration)
                && cwd.is_none()
            {
                *cwd = Some(
                    owner
                        .settings
                        .project_path(root)?
                        .to_str()
                        .ok_or(SettingsError::Invalid("project cwd must be UTF-8"))?
                        .to_owned(),
                );
            }
            if let Some(current) = servers.get(&identity).and_then(|r| r.current.upgrade())
                && current.service.configuration == entry.configuration
                && !matches!(
                    current.state(),
                    server::McpServerState::Failed | server::McpServerState::Closed
                )
            {
                return Ok(current);
            }
            let plugin = McpServerPlugin::new(entry.configuration)?;
            let key = plugin.key.clone();
            let view = owner.context.plugin(capability_plugin(plugin));
            if wait_active(
                &owner.context,
                std::slice::from_ref(&view),
                Duration::from_secs(5),
                &cancel,
            )
            .await
            .is_err()
            {
                view.shutdown().await.map_err(|_| McpServerError::Cleanup)?;
                return Err(McpError::Unavailable);
            }
            // Parent owns the dynamic child; adding an inject back to it would
            // create a readiness cycle. Resolve its provided port after Active.
            let Some(service) = owner.context.get_as::<McpServer>(key) else {
                view.shutdown().await.map_err(|_| McpServerError::Cleanup)?;
                return Err(McpError::Unavailable);
            };
            let connection = Arc::new(McpConnection { view, service });
            let registration = servers.entry(identity).or_default();
            registration.current = Arc::downgrade(&connection);
            registration.generations.push(Arc::downgrade(&connection));
            Ok(connection)
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
        let source = match source {
            SettingsSource::UserMcp => None,
            SettingsSource::ProjectMcp(root) => Some(root),
            SettingsSource::Skills => return Err(McpError::InvalidSource),
        };
        let owner = self.clone();
        self.run(cancel, move |_, cancel| async move {
            let mut servers = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(McpError::Cancelled),
                servers = owner.servers.lock() => servers,
            };
            let identity = (source, id);
            let mut result = Ok(());
            if let Some(server) = servers.remove(&identity) {
                // Own the complete closing set before the first await; callers
                // dropping their leases must not detach our cleanup waits.
                let generations: Vec<_> = server
                    .generations
                    .iter()
                    .filter_map(Weak::upgrade)
                    .collect();
                for generation in generations {
                    if generation.view.shutdown().await.is_err() {
                        result = Err(McpServerError::Cleanup.into());
                    }
                }
            }
            result
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
                    catalog.servers.insert(
                        id,
                        McpServerEntry {
                            source,
                            configuration,
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
    type Deps = (Arc<SettingsService>,);
    fn name(&self) -> &str {
        "Mcp"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (settings,): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Arc::new(McpService {
                settings,
                context: ctx.clone(),
                servers: tokio::sync::Mutex::new(BTreeMap::new()),
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
            ctx.provide_as(TypeKey::of::<McpService>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for McpPlugin {
    fn keys(&self) -> KeysOf<Self> {
        ((),)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Inject failure at the framework cleanup boundary: actual OS child-reap
    // failures are not portable to induce in an external process fixture.
    #[tokio::test]
    async fn disconnect_drains_every_generation_after_a_plugin_cleanup_error() {
        struct CleanupFailure(McpServerPlugin);
        impl TypedPlugin for CleanupFailure {
            type Deps = <McpServerPlugin as TypedPlugin>::Deps;
            fn name(&self) -> &str {
                "McpCleanupFailure"
            }
            fn apply<'a>(
                &'a self,
                ctx: &'a Ctx,
                deps: Self::Deps,
            ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
                Box::pin(async move {
                    ctx.effect(|| {
                        Effect::AsyncDisposer(Box::new(|| {
                            Box::pin(async {
                                Err(CordisError::PluginFailed(Box::new(McpServerError::Cleanup)))
                            })
                        }))
                    })?;
                    self.0.apply(ctx, deps).await
                })
            }
        }
        impl Capability for CleanupFailure {
            fn keys(&self) -> KeysOf<Self> {
                self.0.keys()
            }
        }

        let home = std::env::temp_dir().join(format!("mcp-cleanup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&home).unwrap();
        let home = home.canonicalize().unwrap();
        let configuration = serde_json::json!({"type":"http","url":"http://127.0.0.1:1/mcp"});
        std::fs::write(
            home.join("mcp.json"),
            serde_json::json!({"mcpServers":{"docs":configuration}}).to_string(),
        )
        .unwrap();
        let root = Ctx::root().unwrap();
        let views = [
            root.plugin(dim_agent_runtime::capability_plugin(
                dim_agent_capabilities::file_storage::FileStoragePlugin::new(vec![home.clone()]),
            )),
            root.plugin(dim_agent_runtime::capability_plugin(
                crate::settings::SettingsPlugin::new(0, vec![]),
            )),
            root.plugin(dim_agent_runtime::capability_plugin(McpPlugin)),
        ];
        let cancel = CancellationToken::new();
        wait_active(&root, &views, Duration::from_secs(5), &cancel)
            .await
            .unwrap();
        let directory = root.get::<McpService>().unwrap();
        let mut leases = Vec::new();
        for fails in [true, false] {
            let plugin =
                McpServerPlugin::new(serde_json::from_value(configuration.clone()).unwrap())
                    .unwrap();
            let key = plugin.key.clone();
            let view = if fails {
                directory
                    .context
                    .plugin(capability_plugin(CleanupFailure(plugin)))
            } else {
                directory.context.plugin(capability_plugin(plugin))
            };
            wait_active(
                &directory.context,
                std::slice::from_ref(&view),
                Duration::from_secs(5),
                &cancel,
            )
            .await
            .unwrap();
            leases.push(Arc::new(McpConnection {
                view,
                service: directory.context.get_as(key).unwrap(),
            }));
        }
        directory.servers.lock().await.insert(
            (None, "docs".into()),
            ServerRegistration {
                current: Arc::downgrade(&leases[1]),
                generations: leases.iter().map(Arc::downgrade).collect(),
            },
        );
        let result = directory
            .disconnect(SettingsSource::UserMcp, "docs".into(), &cancel)
            .await;
        let states: Vec<_> = leases.iter().map(|lease| lease.state()).collect();
        let replacement = directory
            .server(None, "docs".into(), &cancel)
            .await
            .unwrap();
        let reused = leases.iter().any(|lease| Arc::ptr_eq(lease, &replacement));
        drop(replacement);
        drop(leases);
        // Framework errors remain reportable even after child unmounting.
        let _ = root.shutdown().await;
        std::fs::remove_dir_all(home).unwrap();
        assert!(result.is_err());
        assert_eq!(states, vec![server::McpServerState::Closed; 2]);
        assert!(
            !reused,
            "a failed disconnect must not preserve a live route"
        );
    }
}
