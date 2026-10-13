use crate::{
    agents::{AgentHost, AgentIndex},
    protocol::{RpcError, TraceOption},
};
use dim_agent_capabilities::trace::{
    TraceHost, TracePlugin,
    archive::{ArchiveContent, TraceArchive},
};
use dim_agent_runtime::{AgentBuilder, Session, SessionConfig};
use rutis::Ctx;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::watch;
use tokio_util::task::TaskTracker;

/// Session extension that records one Session into its trace archive.
const TRACE_EXTENSION: &str = "trace";

/// Registers the trace extension that traced Sessions load.
pub(crate) fn install(builder: &mut AgentBuilder) {
    builder.session_extension(TRACE_EXTENSION, "Trace", |scope, host: &TraceHost| {
        TracePlugin::new(scope, host.clone())
    });
}

/// Trace archive writers of one Agent; each finishes after its Session closes.
pub(crate) struct Traces {
    /// Outlives every Session of the Agent: archive listeners live here.
    ctx: Ctx,
    tasks: TaskTracker,
    sessions: Arc<Mutex<HashMap<String, watch::Receiver<bool>>>>,
}
impl Traces {
    pub fn new(ctx: Ctx) -> Self {
        Self {
            ctx,
            tasks: TaskTracker::default(),
            sessions: Arc::default(),
        }
    }
    pub async fn create(
        &self,
        index: &AgentIndex,
        agent_host: &AgentHost,
        config: SessionConfig,
        version: u32,
        option: TraceOption,
        directory: &Path,
    ) -> std::result::Result<(Session, String), RpcError> {
        let path = match option {
            TraceOption::Enabled(true) => {
                directory.join(format!("{}.ndjson", uuid::Uuid::new_v4()))
            }
            TraceOption::Enabled(false) => {
                return Err(RpcError::invalid("trace must be true or a path"));
            }
            TraceOption::Path(path) => {
                if path.is_empty() {
                    return Err(RpcError::invalid("trace path must not be empty"));
                }
                let path = PathBuf::from(path);
                if path.is_absolute() {
                    path
                } else {
                    Path::new(&config.cwd).join(path)
                }
            }
        };
        if let Some(parent) = path.parent() {
            let mut options = tokio::fs::DirBuilder::new();
            options.recursive(true);
            #[cfg(unix)]
            options.mode(0o700);
            options
                .create(parent)
                .await
                .map_err(|_| RpcError::wire(-32603, "Cannot create trace directory"))?;
        }
        // Never truncate an existing trace supplied by another Session.
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&path)
            .await
            .map_err(|_| RpcError::wire(-32603, "Cannot create trace file"))?;
        let archive = TraceArchive::with_content(file, ArchiveContent::SensitiveContent);
        let host = TraceHost {
            ctx: self.ctx.clone(),
            recorder: archive.recorder(),
        };
        let stored_id = index.prepare_session(agent_host, &config, version).await;
        let agent = &agent_host.agent;
        let created = match stored_id {
            Some((id, prepare)) => {
                agent
                    .create_prepared_session_with_extension(
                        id,
                        config,
                        prepare,
                        TRACE_EXTENSION,
                        host,
                    )
                    .await
            }
            None => {
                agent
                    .create_session_with_extension(config, TRACE_EXTENSION, host)
                    .await
            }
        };
        let session = match created {
            Ok(session) => session,
            Err(error) => {
                finish(archive, &path).await;
                return Err(RpcError::from(error));
            }
        };
        let (done, receiver) = watch::channel(false);
        self.sessions
            .lock()
            .expect("trace index")
            .insert(session.id().to_owned(), receiver);
        let sessions = self.sessions.clone();
        let closing = session.clone();
        let trace_path = path.to_string_lossy().into_owned();
        self.tasks.spawn(async move {
            // Business cleanup failure does not imply lost diagnostic records.
            let _ = closing.closed().await;
            finish(archive, &path).await;
            done.send_replace(true);
            sessions.lock().expect("trace index").remove(closing.id());
        });
        Ok((session, trace_path))
    }
    pub async fn wait_session(&self, id: &str) {
        let rx = self.sessions.lock().expect("trace index").get(id).cloned();
        if let Some(mut rx) = rx {
            while !*rx.borrow_and_update() {
                if rx.changed().await.is_err() {
                    break;
                }
            }
        }
    }
    pub async fn finish(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

async fn finish(archive: TraceArchive, path: &Path) {
    match archive.finish().await {
        Ok(status) if status.incomplete => {
            tracing::warn!(trace_path=%path.display(), "Trace archive is incomplete")
        }
        Ok(_) => {}
        Err(error) => tracing::error!(%error, trace_path=%path.display(), "Trace archive failed"),
    }
}
