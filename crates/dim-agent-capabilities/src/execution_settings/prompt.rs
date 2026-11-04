//! Model-facing execution settings: snapshots and the one `settings` tool.
//!
//! The App Server's `ExecutionSettings` owns the Settings document and
//! provides `ExecutionSettingsHost`. This module appends a snapshot of the
//! effective switches and registers the single `settings` tool. It does not
//! register a Prompt source.
use super::{ExecutionSettings, ExecutionSettingsHost};
use crate::{
    branch_input::append_own,
    host::ToolExecutionContext,
    settings::{SettingsChanged, settings_changes},
    tool_registry::{ToolContribution, ToolHandler, ToolRegistry},
};
use dim_agent_runtime::{Dispatcher, Names, PartRegistry, PartScope, SessionIdentity, Timeline};
use dim_agent_types::*;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use rutis::{Ctx, Error};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;

/// The snapshot source name.
pub const SNAPSHOT_SOURCE: &str = "dim/settings/execution";
/// The Settings source whose committed writes wake the projection.
const SETTINGS_SOURCE: &str = "execution";
/// The tool name.
const TOOL: &str = "settings";
/// The Session plugin name.
pub const SESSION_PART: &str = "SettingsTool";
/// The branch plugin name.
pub const BRANCH_PART: &str = "SettingsSnapshotBranch";

/// Registers the one settings tool and the settings snapshot branch for the
/// process plugin that owns `ExecutionSettingsHost`.
pub fn register_parts(ctx: &Ctx, parts: &Arc<PartRegistry>) -> Result<(), Error> {
    parts.register(ctx, PartScope::Session, SESSION_PART, |mount| {
        Ok(Some(Box::new(SettingsTool::new(mount.names)?)))
    })?;
    parts.register(ctx, PartScope::Branch, BRANCH_PART, |mount| {
        Ok(Some(Box::new(SettingsSnapshotBranch::new(mount.names)?)))
    })
}

fn body(settings: ExecutionSettings) -> String {
    if settings.sandbox {
        format!(
            "Execution sandbox: on; network: {}. New processes only; a running shell keeps its original boundary.",
            if settings.network { "on" } else { "off" }
        )
    } else {
        "Execution sandbox: off. New processes run with the host's normal access.".into()
    }
}

fn snapshot(settings: ExecutionSettings) -> String {
    format!(
        "<dim-state source=\"{SNAPSHOT_SOURCE}\">{}</dim-state>",
        body(settings)
    )
}

fn append(timeline: &Timeline, text: &str) -> dim_agent_runtime::Result<()> {
    append_own(
        timeline,
        InputMessageData {
            blocks: vec![InputBlock::Raw {
                text: text.to_owned(),
                key: None,
                retire: false,
            }],
            ..InputMessageData::default()
        },
        Vec::new(),
    )
    .map(|_| ())
}

fn settings_view(settings: ExecutionSettings) -> Value {
    json!({
        "settings": [
            {
                "name": "execution.sandbox",
                "description": "Run shell commands inside the sandbox.",
                "type": "boolean",
                "value": settings.sandbox,
                "writable": false
            },
            {
                "name": "execution.network",
                "description": "Allow network access for new processes. Only meaningful when the sandbox is on.",
                "type": "boolean",
                "value": settings.network,
                "writable": false
            }
        ]
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    op: String,
    name: Option<String>,
    value: Option<Value>,
}

struct SettingsHandler {
    host: Arc<ExecutionSettingsHost>,
}
impl SettingsHandler {
    async fn answer(&self, call: &ToolCall) -> ToolResult {
        let arguments: Arguments = match serde_json::from_value(Value::Object(call.input.clone())) {
            Ok(arguments) => arguments,
            Err(error) => return ToolResult::error(format!("invalid settings arguments: {error}")),
        };
        let settings = match self.host.0.read(CancellationToken::new()).await {
            Ok(settings) => settings,
            Err(error) => return ToolResult::error(format!("execution settings: {error}")),
        };
        if arguments.op == "set" && arguments.value.is_none() {
            return ToolResult::error("settings set requires value");
        }
        match arguments.op.as_str() {
            "list" => ToolResult::text(settings_view(settings).to_string()),
            "get" => {
                let Some(name) = arguments.name else {
                    return ToolResult::error("settings get requires name");
                };
                let view = settings_view(settings);
                let Some(setting) = view["settings"]
                    .as_array()
                    .and_then(|settings| settings.iter().find(|item| item["name"] == name))
                else {
                    return ToolResult::error(format!(
                        "unknown setting {name}; call list"
                    ));
                };
                ToolResult::text(setting.to_string())
            }
            "set" => {
                let Some(name) = arguments.name else {
                    return ToolResult::error("settings set requires name");
                };
                if !settings_view(settings)["settings"]
                    .as_array()
                    .is_some_and(|settings| settings.iter().any(|item| item["name"] == name))
                {
                    return ToolResult::error(format!("unknown setting {name}; call list"));
                }
                ToolResult::error(format!("not_writable: {name} is read-only"))
            }
            other => ToolResult::error(format!(
                "unknown settings op {other}; use list, get or set"
            )),
        }
    }
}
impl ToolHandler for SettingsHandler {
    fn execute(
        &self,
        call: ToolCall,
        _: ToolExecutionContext,
        _: CancellationToken,
    ) -> crate::host::HostFuture<'_, ToolResult> {
        Box::pin(async move { Ok(self.answer(&call).await) })
    }
}

fn definition() -> ToolDefinition {
    ToolDefinition {
        name: TOOL.into(),
        description: "Read or change the user's settings. Call op=list first to see each setting's name, meaning, current value, type, and whether it is writable. op=get reads one setting. op=set changes a writable setting; read-only settings return not_writable. Never guess a setting name.".into(),
        input_schema: serde_json::from_value(json!({
            "type": "object",
            "properties": {
                "op": {
                    "type": "string",
                    "enum": ["list", "get", "set"],
                    "description": "list: show available settings. get: read one. set: change one."
                },
                "name": {
                    "type": "string",
                    "description": "A setting name returned by list."
                },
                "value": {
                    "description": "The new value for set. list shows the required type."
                }
            },
            "required": ["op"],
            "additionalProperties": false
        }))
        .expect("valid settings schema"),
        source: ToolSource::Extension,
        risk: None,
        metadata: None,
        file_changes: None,
    }
}

struct SettingsTool {
    host: Key<ExecutionSettingsHost>,
    tools: Key<ToolRegistry>,
}
impl SettingsTool {
    fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            host: names.key()?,
            tools: names.key()?,
        })
    }
}
impl DynPlugin for SettingsTool {
    fn name(&self) -> &str {
        SESSION_PART
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.host.erase(), self.tools.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let host = ctx.require_at(&self.host)?;
            let tools = ctx.require_at(&self.tools)?;
            tools.register(
                ctx,
                ToolContribution {
                    definition: definition(),
                    handler: Arc::new(SettingsHandler { host }),
                },
            )?;
            Ok(Effect::Done)
        })
    }
}

struct SettingsSnapshotBranch {
    host: Key<ExecutionSettingsHost>,
    identity: Key<SessionIdentity>,
    timeline: Key<Timeline>,
    dispatcher: Key<Dispatcher>,
}
impl SettingsSnapshotBranch {
    fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            host: names.key()?,
            identity: names.key()?,
            timeline: names.key()?,
            dispatcher: names.key()?,
        })
    }
}
impl DynPlugin for SettingsSnapshotBranch {
    fn name(&self) -> &str {
        BRANCH_PART
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.host.erase(),
            self.identity.erase(),
            self.timeline.erase(),
            self.dispatcher.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let host = ctx.require_at(&self.host)?;
            let identity = ctx.require_at(&self.identity)?;
            let timeline = ctx.require_at(&self.timeline)?;
            let dispatcher = ctx.require_at(&self.dispatcher)?;
            let last = Arc::new(Mutex::new(None));
            let dirty = Arc::new(AtomicBool::new(false));

            let initial = match host.0.read(CancellationToken::new()).await {
                Ok(settings) => snapshot(settings),
                Err(error) => {
                    tracing::warn!(%error, "execution settings were not read");
                    String::new()
                }
            };
            if !initial.is_empty() {
                if let Err(error) = append(&timeline, &initial) {
                    tracing::warn!(%error, "execution settings were not appended");
                }
                *last.lock().expect("settings body") = Some(initial);
            }

            let session = identity.session_id.clone();
            let changed_host = host.clone();
            let changed_timeline = timeline.clone();
            let changed_last = last.clone();
            let changed_dirty = dirty.clone();
            ctx.listen::<SettingsChanged>()
                .name(settings_changes(SETTINGS_SOURCE))
                .on(move |event, _| {
                    let origin = event.origin.clone();
                    let host = changed_host.clone();
                    let timeline = changed_timeline.clone();
                    let last = changed_last.clone();
                    let dirty = changed_dirty.clone();
                    let session = session.clone();
                    Box::pin(async move {
                        if origin.as_deref() != Some(session.as_str()) {
                            dirty.store(true, Ordering::SeqCst);
                            return Ok(None);
                        }
                        match host.0.read(CancellationToken::new()).await {
                            Ok(settings) => {
                                let current = snapshot(settings);
                                let mut last = last.lock().expect("settings body");
                                if last.as_deref() != Some(current.as_str()) {
                                    if let Err(error) = append(&timeline, &current) {
                                        tracing::warn!(%error, "execution settings were not appended");
                                    } else {
                                        *last = Some(current);
                                    }
                                }
                            }
                            Err(error) => tracing::warn!(%error, "execution settings were not read"),
                        }
                        Ok(None)
                    })
                })?;

            let turn_host = host.clone();
            let turn_timeline = timeline.clone();
            let turn_last = last;
            let turn_dirty = dirty;
            dispatcher.subscribe(
                ctx,
                "SettingsSnapshot",
                &["turn/start"],
                move |_| {
                    if !turn_dirty.swap(false, Ordering::SeqCst) {
                        return Box::pin(async { Ok(()) });
                    }
                    let host = turn_host.clone();
                    let timeline = turn_timeline.clone();
                    let last = turn_last.clone();
                    Box::pin(async move {
                        if let Ok(settings) = host.0.read(CancellationToken::new()).await {
                            let current = snapshot(settings);
                            let mut last = last.lock().expect("settings body");
                            if last.as_deref() != Some(current.as_str()) {
                                if let Err(error) = append(&timeline, &current) {
                                    tracing::warn!(%error, "execution settings were not appended");
                                } else {
                                    *last = Some(current);
                                }
                            }
                        }
                        Ok(())
                    })
                },
            )?;
            Ok(Effect::Done)
        })
    }
}
