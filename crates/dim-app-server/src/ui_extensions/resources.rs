use dim_agent_capabilities::file_storage::{FileReader, FileStorage};
use dim_agent_runtime::{Names, PublicService, SessionIdentity, SessionLifecycle, SessionPhase};
use rutis::{
    Ctx,
    host::{BoxFuture, CtxHost, DynPlugin, Effect, InstanceId, Key, TypeKey},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio_util::{
    sync::CancellationToken,
    task::{TaskTracker, task_tracker::TaskTrackerToken},
};

#[derive(Debug, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct ResourceError {
    pub code: String,
    pub message: String,
}
impl ResourceError {
    fn service(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    fn invalid(message: &str) -> Self {
        Self::service("invalid_params", message)
    }
}

#[derive(Default)]
pub(super) struct Reads {
    gate: Mutex<()>,
    pub stop: CancellationToken,
    tasks: TaskTracker,
}
impl Reads {
    pub fn new(stop: CancellationToken) -> Self {
        Self {
            stop,
            ..Self::default()
        }
    }
    pub fn enter(&self) -> Result<TaskTrackerToken, ResourceError> {
        let _gate = self.gate.lock().unwrap();
        if self.stop.is_cancelled() {
            return Err(gone());
        }
        Ok(self.tasks.token())
    }
    pub async fn close(&self) {
        {
            let _gate = self.gate.lock().unwrap();
            self.stop.cancel();
            self.tasks.close();
        }
        self.tasks.wait().await;
    }
}

pub(crate) struct Read {
    pub file: FileReader,
    pub stop: CancellationToken,
    pub token: TaskTrackerToken,
    pub media_type: String,
    pub attachment: bool,
    pub etag: Option<String>,
}

struct Published {
    owner: InstanceId,
    file: Mutex<Option<FileReader>>,
    reads: Reads,
    media_type: String,
    attachment: bool,
}
impl Published {
    async fn close(&self) {
        self.reads.close().await;
        self.file.lock().unwrap().take();
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceRef {
    pub session_id: String,
    pub id: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishFile {
    pub path: String,
    pub media_type: String,
}

struct Registration {
    resource: Arc<Published>,
    cleanup: rutis::Disposer,
}

pub struct UiResources {
    session_id: String,
    lifecycle: Arc<SessionLifecycle>,
    files: Arc<FileStorage>,
    resources: Arc<Mutex<HashMap<String, Registration>>>,
    reads: Arc<Reads>,
}
impl PublicService for UiResources {}

#[rutis::service]
impl UiResources {
    /// Publishes a finalized regular file inside an authorized FileStorage root.
    /// The calling part owns the reference until withdrawal or unload.
    pub async fn publish_file(
        &self,
        caller: &Ctx,
        request: PublishFile,
    ) -> Result<ResourceRef, ResourceError> {
        let _operation = self.reads.enter()?;
        if self.lifecycle.phase() >= SessionPhase::Closing {
            return Err(gone());
        }
        let path = self
            .files
            .resolve(Path::new(&request.path))
            .map_err(file_error)?;
        let cancel = caller.cancellation_token().child_token();
        let _cancel = cancel.clone().drop_guard();
        let file = tokio::select! {
            _ = self.reads.stop.cancelled() => return Err(gone()),
            _ = self.lifecycle.closing() => return Err(gone()),
            file = self.files.open_read(&path, &cancel) => file.map_err(file_error)?,
        };
        let inline = matches!(
            request.media_type.as_str(),
            "image/png"
                | "image/jpeg"
                | "image/gif"
                | "image/webp"
                | "image/avif"
                | "audio/mpeg"
                | "audio/ogg"
                | "audio/wav"
                | "audio/mp4"
                | "audio/webm"
                | "video/mp4"
                | "video/webm"
                | "video/ogg"
                | "text/plain"
                | "application/pdf"
        );
        let published = Arc::new(Published {
            owner: caller.instance(),
            file: Mutex::new(Some(file)),
            reads: Reads::new(self.reads.stop.child_token()),
            media_type: if inline {
                request.media_type
            } else {
                "application/octet-stream".into()
            },
            attachment: !inline,
        });
        let id = uuid::Uuid::new_v4().to_string();
        let resources = self.resources.clone();
        let cleanup = published.clone();
        let key = id.clone();
        let mut entries = self.resources.lock().unwrap();
        if self.reads.stop.is_cancelled()
            || self.lifecycle.phase() >= SessionPhase::Closing
            || caller.cancellation_token().is_cancelled()
        {
            return Err(gone());
        }
        let disposer = caller
            .effect(move || async move {
                cleanup.close().await;
                resources.lock().unwrap().remove(&key);
                Ok(())
            })
            .map_err(|error| ResourceError::service("resource_closed", error.to_string()))?;
        entries.insert(
            id.clone(),
            Registration {
                resource: published,
                cleanup: disposer,
            },
        );
        Ok(ResourceRef {
            session_id: self.session_id.clone(),
            id,
        })
    }

    /// Withdraws only a reference this calling part published; waits for reads to end.
    pub async fn withdraw(
        &self,
        caller: &Ctx,
        reference: ResourceRef,
    ) -> Result<(), ResourceError> {
        if reference.session_id != self.session_id {
            return Err(gone());
        }
        let resource = {
            let entries = self.resources.lock().unwrap();
            match entries.get(&reference.id) {
                Some(entry) if entry.resource.owner != caller.instance() => {
                    return Err(ResourceError::invalid("Resource belongs to another part"));
                }
                Some(entry) => Some(entry.resource.clone()),
                None => None,
            }
        };
        if let Some(resource) = resource {
            resource.close().await;
            let entry = self.resources.lock().unwrap().remove(&reference.id);
            if let Some(entry) = entry {
                entry.cleanup.dispose().await.map_err(file_error)?;
            }
        }
        Ok(())
    }
}
impl UiResources {
    pub(crate) fn read(&self, id: &str) -> Result<Read, ResourceError> {
        if self.lifecycle.phase() != SessionPhase::Active {
            return Err(gone());
        }
        let entries = self.resources.lock().unwrap();
        let entry = &entries.get(id).ok_or_else(gone)?.resource;
        let token = entry.reads.enter()?;
        let file = entry.file.lock().unwrap().clone().ok_or_else(gone)?;
        Ok(Read {
            file,
            stop: entry.reads.stop.clone(),
            token,
            media_type: entry.media_type.clone(),
            attachment: entry.attachment,
            etag: None,
        })
    }
    async fn close(&self) {
        self.reads.close().await;
        let entries = std::mem::take(&mut *self.resources.lock().unwrap());
        for entry in entries.into_values() {
            if let Err(error) = entry.cleanup.dispose().await {
                tracing::warn!(%error, "UI resource cleanup failed");
            }
        }
    }
}

pub(super) fn gone() -> ResourceError {
    ResourceError::service("resource_gone", "UI resource is unavailable")
}
pub(super) fn file_error(error: impl std::fmt::Display) -> ResourceError {
    ResourceError::service("resource_unavailable", error.to_string())
}

pub(super) struct UiResourcesPlugin {
    identity: Key<SessionIdentity>,
    lifecycle: Key<SessionLifecycle>,
    files: Key<FileStorage>,
    key: Key<UiResources>,
}
impl UiResourcesPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            identity: names.key()?,
            lifecycle: names.key()?,
            files: names.key()?,
            key: names.key()?,
        })
    }
}
impl DynPlugin for UiResourcesPlugin {
    fn name(&self) -> &str {
        "UiResources"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.identity.erase(),
            self.lifecycle.erase(),
            self.files.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let lifecycle = ctx.require_at(&self.lifecycle)?;
            let service = Arc::new(UiResources {
                session_id: ctx.require_at(&self.identity)?.session_id.clone(),
                lifecycle: lifecycle.clone(),
                files: ctx.require_at(&self.files)?,
                resources: Arc::default(),
                reads: Arc::new(Reads::new(ctx.cancellation_token())),
            });
            let cleanup = service.clone();
            let stop = ctx.cancellation_token();
            let running = service.clone();
            let task = tokio::spawn(async move {
                tokio::select! {
                    _ = stop.cancelled() => {},
                    _ = async { if lifecycle.activated().await { lifecycle.closing().await; } } => {},
                }
                running.close().await;
            });
            ctx.effect(move || async move {
                cleanup.close().await;
                let _ = task.await;
                Ok(())
            })?;
            ctx.provide_at(self.key.clone(), service)?;
            Ok(Effect::Done)
        })
    }
}
