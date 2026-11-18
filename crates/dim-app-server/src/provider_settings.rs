//! The `provider` settings source: the User-Agent dim puts on its model
//! requests, when the user replaces the built-in one.
//!
//! Provider owns its process configuration
//! (`docs/design/appserver/provider-management.md` §2), so this source belongs
//! here. It holds no Provider record: a record's own `headers` stay the
//! record's, and a User-Agent there still wins for that Provider alone.
use crate::settings::{
    Settings, SettingsChanged, SettingsScope, SourceDeclaration, settings_changes,
};
use dim_agent_runtime::Names;
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, ErrorKind, Key, TypeKey};
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// The source name, as `settings.update` names it.
const SOURCE: &str = "provider";
/// The document file under the settings root.
const USER_FILE: &str = "provider.json";

fn source_declaration() -> SourceDeclaration {
    SourceDeclaration {
        source: SOURCE.into(),
        user_file: USER_FILE.into(),
        project_file: None,
        schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "userAgent": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 512,
                    "pattern": "^[ -~]+$"
                }
            }
        }),
        user_only: vec!["/userAgent".into()],
    }
}

/// The document the source stores. `userAgent` replaces dim's whole User-Agent
/// string; `DIM_USER_AGENT` wins over it when that is set.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    #[serde(default)]
    user_agent: Option<String>,
}

pub struct ProviderSettingsPlugin {
    settings: Key<Settings>,
}

impl ProviderSettingsPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            settings: names.key()?,
        })
    }
}

impl DynPlugin for ProviderSettingsPlugin {
    fn name(&self) -> &str {
        "ProviderSettings"
    }

    fn injects(&self) -> Vec<TypeKey> {
        vec![self.settings.erase()]
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let settings = ctx.require_at(&self.settings)?;
            settings.register(ctx, source_declaration())?;
            if let Err(error) = apply(&settings).await {
                return Err(rutis::Error::from(ErrorKind::PluginFailed(error)));
            }
            if let Err(error) = ctx.effect(|| async {
                dim_agent_aimux::set_configured_user_agent(None);
                Ok(())
            }) {
                dim_agent_aimux::set_configured_user_agent(None);
                return Err(error);
            }
            let settings = settings.clone();
            ctx.listen::<SettingsChanged>()
                .name(settings_changes(SOURCE))
                .on(move |_, _| {
                    let settings = settings.clone();
                    async move {
                        if let Err(error) = apply(&settings).await {
                            tracing::warn!(%error, "the model request User-Agent was not updated");
                        }
                        Ok(None)
                    }
                })?;
            Ok(Effect::Done)
        })
    }
}

/// Publishes the document's value. A missing or unreadable document leaves dim
/// on its built-in User-Agent.
async fn apply(settings: &Settings) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cancel = CancellationToken::new();
    let snapshot = settings.read(SOURCE, SettingsScope::User, &cancel).await?;
    let document: Document = serde_json::from_value(snapshot.document)?;
    dim_agent_aimux::set_configured_user_agent(document.user_agent);
    Ok(())
}
