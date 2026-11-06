//! The built-in Modality adapter: the media operations aimux serves, executed
//! for one task at a time.
//!
//! The vendor is the catalog provider's identity ([`vendor`]); the connection
//! is Provider's per-call resolution; nothing is kept between calls except the
//! job a blocking operation is still running.
//!
//! A blocking operation does its whole call in `wait` and returns no handle,
//! so a restart leaves it `interrupted` — A.2's statement for an adapter with
//! no handle.

mod job;
mod media;
pub mod vendor;

use aimux_core::AiMuxError;
use aimux_provider_utils::FetchFunction;
use dim_agent_aimux::MediaTransport;
use dim_agent_capabilities::host::{ModalityConnection, ModelConnection};
use dim_agent_runtime::AgentBuilder;
use dim_agent_types::ModelId;
use dim_modality::{
    AdapterError, AdapterFuture, AdapterOperation, AdapterOutput, AdapterProgress, Job,
    ModalityAdapter, ModalityAdapterService, ProgressSink, RemoteCancel, Submitted,
};
use rutis::Ctx;
use rutis::host::{BoxFuture, DynPlugin, Effect};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use vendor::Operation;

/// How long a blocking job waits in the map for the `wait` that removes it.
///
/// The engine can skip `wait` after a successful `submit`: a cancel during the
/// submit phase, an adapter that turned unavailable or a connection failure,
/// or a cancel that wins the select before `wait` is polled. Such a job is
/// never looked at again, so it must expire instead of living with the
/// process.
const MAX_PENDING_AGE: Duration = Duration::from_secs(10 * 60);
/// The most blocking jobs the map keeps: an age bound alone still lets a
/// burst of submits hold every job for the full age.
const MAX_PENDING_JOBS: usize = 64;

/// One submitted blocking job, waiting for its `wait`.
struct Pending {
    job: Job,
    since: Instant,
}

/// The process-scoped `modalityAdapter` the product serves.
pub struct MediaAdapter {
    transport: media::Transport,
    /// The jobs of blocking operations this generation is running: their
    /// `wait` carries no handle, so the job is kept from `submit` to `wait`
    /// and never across a generation. The map is bounded because the engine
    /// can skip `wait`; see [`MAX_PENDING_AGE`].
    jobs: Mutex<HashMap<String, Pending>>,
}

impl MediaAdapter {
    /// The adapter the product installs: the connection's own transport.
    pub fn new() -> Self {
        Self {
            transport: media::Transport::Shared(MediaTransport::new()),
            jobs: Mutex::new(HashMap::new()),
        }
    }
    /// An adapter whose requests are answered by `fetch`, for tests.
    pub fn scripted(fetch: FetchFunction) -> Self {
        Self {
            transport: media::Transport::Fixed(fetch),
            jobs: Mutex::new(HashMap::new()),
        }
    }
    /// The vendor of a model's provider, or `unavailable` for a created
    /// provider this build has no row for.
    fn vendor(&self, model: &ModelId) -> Result<&'static vendor::Vendor, AdapterError> {
        vendor::vendor(&model.provider_id).ok_or_else(|| {
            AdapterError::unavailable(format!(
                "No adapter row serves the provider `{}`",
                model.provider_id
            ))
        })
    }
    /// Holds a blocking job until its `wait`, within the bounds above.
    fn keep(&self, job: Job, now: Instant) {
        let mut jobs = self.jobs.lock().unwrap();
        jobs.retain(|_, pending| now.saturating_duration_since(pending.since) < MAX_PENDING_AGE);
        jobs.insert(job.task_id.clone(), Pending { job, since: now });
        while jobs.len() > MAX_PENDING_JOBS {
            let Some(oldest) = jobs
                .iter()
                .min_by_key(|(_, pending)| pending.since)
                .map(|(task_id, _)| task_id.clone())
            else {
                break;
            };
            jobs.remove(&oldest);
        }
    }
}

impl Default for MediaAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ModalityAdapter for MediaAdapter {
    fn available(&self) -> bool {
        true
    }

    fn operations(&self, model: &ModelId) -> Vec<AdapterOperation> {
        let Some(vendor) = vendor::vendor(&model.provider_id) else {
            return Vec::new();
        };
        let edit = media::serves_edit(vendor, &model.model_id);
        let mut operations = Vec::new();
        for (kind, category, operation) in vendor
            .statements(edit)
            .into_iter()
            .filter(|(kind, _, _)| matches!(kind, Operation::Image))
        {
            let schema = match (kind, operation) {
                (Operation::Image, "edit") => image_edit_schema(),
                (Operation::Image, _) => image_generate_schema(),
                (_, _) => unreachable!("only an image statement is offered"),
            };
            operations.push(AdapterOperation {
                category: category.to_owned(),
                operation: operation.to_owned(),
                params_schema: schema,
            });
        }
        operations
    }

    fn submit(
        &self,
        job: Job,
        connection: ModalityConnection,
        cancel: CancellationToken,
    ) -> AdapterFuture<'_, Submitted> {
        Box::pin(async move {
            let vendor = self.vendor(&job.model)?;
            let kind = kind_of(&job)?;
            if !vendor.kinds.serves(kind) {
                return Err(AdapterError::unavailable(format!(
                    "The {} provider serves no {}",
                    vendor.catalog, job.operation
                )));
            }
            if job.operation == "edit" && !media::serves_edit(vendor, &job.model.model_id) {
                return Err(AdapterError::unavailable(format!(
                    "The {} provider serves no image.edit for `{}`",
                    vendor.catalog, job.model.model_id
                )));
            }
            let _ = (cancel, connection);
            self.keep(job, Instant::now());
            Ok(Submitted::default())
        })
    }

    fn wait(
        &self,
        task_id: &str,
        _handle: Option<&str>,
        connection: ModalityConnection,
        progress: ProgressSink,
        cancel: CancellationToken,
    ) -> AdapterFuture<'_, AdapterOutput> {
        // The record's strings are borrowed from the caller; the future owns
        // them so it never outlives the call's arguments.
        let task_id = task_id.to_owned();
        Box::pin(async move {
            // A blocking operation's job belongs to this generation only: a
            // recorded task never reaches here with an empty handle (A.6).
            let pending = self.jobs.lock().unwrap().remove(&task_id).ok_or_else(|| {
                AdapterError::provider_error(format!(
                    "This generation never started the task {task_id}"
                ))
            })?;
            let job = pending.job;
            let vendor = self.vendor(&job.model)?;
            let secrets = Secrets::of(&connection);
            match kind_of(&job)? {
                Operation::Image => {
                    progress.send(AdapterProgress {
                        fraction: None,
                        message: Some("Generating the image".into()),
                    });
                    let provider =
                        media::provider(&connection, vendor, Operation::Image, &self.transport)?;
                    let model = media::image_model(&provider, &connection.model)?;
                    let options = job::image_options(&job, vendor)?;
                    let result = aimux_core::image_model::generate_image(&*model, options)
                        .await
                        .map_err(|error| failure(error, &cancel, &secrets))?;
                    let files =
                        job::image_output(result, &job.work_dir, job::requested_format(&job))?;
                    Ok(AdapterOutput { files, data: None })
                }
                _ => Err(AdapterError::provider_error(
                    "No such operation is served in this build",
                )),
            }
        })
    }

    fn cancel_remote(
        &self,
        _handle: &str,
        _connection: ModalityConnection,
    ) -> AdapterFuture<'_, RemoteCancel> {
        // aimux exposes no cancel for a started provider job: the provider
        // finishes and bills it whatever this build does (A.2 `unsupported`).
        Box::pin(async { Ok(RemoteCancel::Unsupported) })
    }
}

/// The values one call must never write into a record: the credential, and
/// the header values that may carry it.
struct Secrets(Vec<String>);

impl Secrets {
    fn of(connection: &ModalityConnection) -> Self {
        let mut values = Vec::new();
        if let ModelConnection::Http(http) = &connection.resolved.connection {
            for value in [&http.bearer, &http.api_key].into_iter().flatten() {
                if !value.is_empty() {
                    values.push(value.clone());
                }
            }
            for value in http.headers.values() {
                if !value.is_empty() && !values.contains(value) {
                    values.push(value.clone());
                }
            }
        }
        Self(values)
    }
    /// The message with every URL and every secret removed.
    fn redact(&self, message: &str) -> String {
        let mut text = strip_urls(message);
        for secret in &self.0 {
            text = text.replace(secret.as_str(), "[redacted]");
        }
        text
    }
}

/// `message` with every `http(s)://…` token replaced by `[url]`.
fn strip_urls(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while !rest.is_empty() {
        let start = rest
            .char_indices()
            .find(|(index, _)| {
                let tail = &rest[*index..];
                tail.starts_with("https://") || tail.starts_with("http://")
            })
            .map(|(index, _)| index);
        match start {
            Some(index) => {
                out.push_str(&rest[..index]);
                let tail = &rest[index..];
                let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
                out.push_str("[url]");
                rest = &tail[end..];
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

/// One aimux failure as the adapter reports it, with nothing a record may not
/// hold: [`dim_agent_aimux::failure`] is the one-line classification a chat
/// call already reports, and the adapter scrubs what a record must not store.
fn failure(error: AiMuxError, cancel: &CancellationToken, secrets: &Secrets) -> AdapterError {
    let unavailable = matches!(
        error,
        AiMuxError::NoSuchModel { .. }
            | AiMuxError::NoSuchProvider { .. }
            | AiMuxError::UnsupportedFunctionality(_)
    );
    let message = secrets.redact(&dim_agent_aimux::failure(error, cancel).to_string());
    if unavailable {
        AdapterError::unavailable(message)
    } else {
        AdapterError::provider_error(message)
    }
}

/// The aimux operation a job's `<category>.<operation>` names.
fn kind_of(job: &Job) -> Result<Operation, AdapterError> {
    match (job.category.as_str(), job.operation.as_str()) {
        ("image", "generate") | ("image", "edit") => Ok(Operation::Image),
        (category, operation) => Err(AdapterError::invalid_input(format!(
            "The aimux adapter serves no {category}.{operation}"
        ))),
    }
}

fn image_generate_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "prompt": {"type": "string", "minLength": 1},
            "n": {"type": "integer", "minimum": 1, "maximum": 4},
            "size": {"type": "string", "pattern": "^[0-9]+x[0-9]+$"},
            "aspectRatio": {"type": "string", "pattern": "^[0-9]+:[0-9]+$"},
            "seed": {"type": "integer", "minimum": 0},
            "outputFormat": {"enum": ["png", "jpeg", "webp"]},
            "providerOptions": {"type": "object"}
        },
        "required": ["prompt"],
        "additionalProperties": false
    })
}

fn image_edit_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "prompt": {"type": "string"},
            "n": {"type": "integer", "minimum": 1, "maximum": 4},
            "size": {"type": "string", "pattern": "^[0-9]+x[0-9]+$"},
            "seed": {"type": "integer", "minimum": 0},
            "outputFormat": {"enum": ["png", "jpeg", "webp"]},
            "mask": {"type": "string"},
            "providerOptions": {"type": "object"}
        },
        "additionalProperties": false
    })
}

/// The plugin that provides the adapter at the process scope.
struct AdapterPlugin(Arc<MediaAdapter>);

impl DynPlugin for AdapterPlugin {
    fn name(&self) -> &str {
        "ModalityAimux"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, rutis::Error>> {
        Box::pin(async move {
            ctx.provide(ModalityAdapterService(self.0.clone()))?;
            Ok(Effect::Done)
        })
    }
}

/// Installs the built-in adapter as a process plugin.
///
/// The host registers the service name first, and adds this plugin before
/// Modality's own: Modality reads the service once, when it starts.
pub fn install(builder: &mut AgentBuilder) {
    dim_agent_runtime::names::register::<ModalityAdapterService>(builder.catalog_mut(), None);
    builder.plugin(|_| {
        Ok(Box::new(AdapterPlugin(Arc::new(MediaAdapter::new()))) as Box<dyn DynPlugin>)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn pending_job(task_id: &str) -> Job {
        Job {
            task_id: task_id.to_owned(),
            category: "image".to_owned(),
            operation: "generate".to_owned(),
            model: ModelId {
                provider_id: "openai".to_owned(),
                model_id: "gpt-image-2".to_owned(),
            },
            params: json!({"prompt": "a cat"}),
            files: Vec::new(),
            work_dir: PathBuf::from("/tmp"),
        }
    }

    #[test]
    fn an_unwaited_job_is_swept_after_the_age_bound() {
        let adapter = MediaAdapter::new();
        let now = Instant::now();
        adapter.keep(
            pending_job("mt_old"),
            now - MAX_PENDING_AGE - Duration::from_secs(1),
        );
        adapter.keep(pending_job("mt_new"), now);
        let jobs = adapter.jobs.lock().unwrap();
        assert!(!jobs.contains_key("mt_old"), "the aged job was swept");
        assert!(jobs.contains_key("mt_new"), "the fresh job is kept");
    }

    #[test]
    fn the_pending_map_keeps_at_most_the_job_bound() {
        let adapter = MediaAdapter::new();
        let now = Instant::now();
        for index in 0..=MAX_PENDING_JOBS {
            adapter.keep(
                pending_job(&format!("mt_{index}")),
                now + Duration::from_millis(index as u64),
            );
        }
        let jobs = adapter.jobs.lock().unwrap();
        assert_eq!(jobs.len(), MAX_PENDING_JOBS);
        assert!(!jobs.contains_key("mt_0"), "the oldest job made room");
        assert!(jobs.contains_key(&format!("mt_{MAX_PENDING_JOBS}")));
    }
}
