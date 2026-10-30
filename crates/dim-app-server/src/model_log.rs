//! The user's switch for recording model wire exchanges
//! (`docs/design/appserver/model-log.md`). The recording itself is aimux's
//! RFC-0023 format, written under the home root.
use crate::settings::{
    SettingsChanged, SettingsScope, SettingsService, SourceDeclaration, settings_changes,
};
use dim_agent_capabilities::file_storage::FileStorage;
use dim_agent_runtime::Names;
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, ErrorKind, Key, TypeKey};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const SOURCE: &str = "model-log";

fn source_declaration() -> SourceDeclaration {
    SourceDeclaration {
        source: SOURCE.into(),
        user_file: "model-log.json".into(),
        project_file: None,
        schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "enabled": { "type": "boolean", "default": false }
            }
        }),
        user_only: Vec::new(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    enabled: bool,
}

/// What the last apply installed. `Never` makes the first read act even when
/// it resolves to `None`, so a recorder left by an earlier process state is
/// cleared.
#[derive(Clone, PartialEq)]
enum Applied {
    Never,
    At(Option<PathBuf>),
}

pub struct ModelLogPlugin {
    settings: Key<SettingsService>,
    files: Key<FileStorage>,
    home: usize,
    /// The directory the recorder currently writes to, so a re-read of the
    /// same document keeps the writer instead of opening a second one.
    applied: Arc<Mutex<Applied>>,
}

impl ModelLogPlugin {
    pub fn new(names: &Names<'_>, home: usize) -> rutis::Result<Self> {
        Ok(Self {
            settings: names.key()?,
            files: names.key()?,
            home,
            applied: Arc::new(Mutex::new(Applied::Never)),
        })
    }
}

impl DynPlugin for ModelLogPlugin {
    fn name(&self) -> &str {
        "ModelLog"
    }

    fn injects(&self) -> Vec<TypeKey> {
        vec![self.settings.erase(), self.files.erase()]
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let settings = ctx.require_at(&self.settings)?;
            let files = ctx.require_at(&self.files)?;
            settings.register_declared(ctx, source_declaration())?;
            if let Err(error) = apply(&settings, &files, self.home, &self.applied).await {
                return Err(rutis::Error::from(ErrorKind::PluginFailed(error)));
            }
            if let Err(error) = ctx.effect(|| async {
                let _ = dim_agent_aimux::recording::set_request_recording(None);
                Ok(())
            }) {
                let _ = dim_agent_aimux::recording::set_request_recording(None);
                return Err(error);
            }
            let settings = settings.clone();
            let files = files.clone();
            let applied = self.applied.clone();
            let home = self.home;
            ctx.listen::<SettingsChanged>()
                .name(settings_changes(SOURCE))
                .on(move |_, _| {
                    let settings = settings.clone();
                    let files = files.clone();
                    let applied = applied.clone();
                    async move {
                        if let Err(error) = apply(&settings, &files, home, &applied).await {
                            tracing::warn!(%error, "model request recording was not updated");
                        }
                        Ok(None)
                    }
                })?;
            Ok(Effect::Done)
        })
    }
}

/// Reads the source and starts, stops or keeps the recorder. Off when the
/// document is missing or disabled; the home root holds
/// `model-log/recordings.jsonl`. The lock serializes reads and swaps, so two
/// `settings.changed` handlers cannot apply an older document after a newer
/// one.
async fn apply(
    settings: &SettingsService,
    files: &FileStorage,
    home: usize,
    applied: &Mutex<Applied>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let root = files.root_path(home)?;
    let cancel = CancellationToken::new();
    let snapshot = settings
        .read_declared(SOURCE, SettingsScope::User, &cancel)
        .await?;
    let document: Document = serde_json::from_value(snapshot.document)?;
    let directory = document.enabled.then(|| root.join("model-log"));
    let mut applied = applied.lock().await;
    if *applied == Applied::At(directory.clone()) {
        return Ok(());
    }
    dim_agent_aimux::recording::set_request_recording(directory.as_deref())?;
    *applied = Applied::At(directory);
    Ok(())
}
