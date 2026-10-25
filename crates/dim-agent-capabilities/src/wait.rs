//! The `wait` tool, see `docs/architecture/rust-agent/plugins/Wait.md`; definition, rules
//! and content copy TS `packages/agent/src/tool/wait-plugin.ts`.
//!
//! `WaitPlugin` (Session) contributes the tool; `WaitBranchPlugin` provides
//! `B:Wait`, which records the background completion notifications and
//! steers dispatched on its branch between `turn/start` and `turn/end` (the
//! input that opened the turn comes before `turn/start`, so it never ends a
//! wait, as TS only commits inputs of a running turn).
use crate::{
    host::{HostError, HostFuture, ToolExecutionContext},
    timeline_tree::TreeCommands,
    tool_registry::{ToolContribution, ToolHandler, ToolRegistry, definition},
    workspace_tools::background::{WAKE_TASK_TAG, tagged},
};
use dim_agent_runtime::{AgentBuilder, AgentError, CAPABILITY_RELOADING, Delivery, Dispatcher};
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, DepKey, Effect, Keyed, TypedPlugin};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const WAIT_TOOL: &str = "wait";
const DEFAULT_TIMEOUT_MS: u64 = 300_000;

/// Registers the Session tool and the branch part.
pub fn install(builder: &mut AgentBuilder) {
    builder
        .session_part(|names| names.part(WaitPlugin))
        .branch_part(|names| {
            names.part(WaitBranchPlugin {
                wait: names.key::<Wait>()?,
            })
        });
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Arrival {
    BackgroundTask(String),
    Steer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitResult {
    Notification {
        task_ids: Vec<String>,
        waited_ms: u64,
    },
    UserSteer {
        waited_ms: u64,
    },
    Timeout {
        waited_ms: u64,
    },
    Interrupted {
        waited_ms: u64,
    },
}

/// Another `wait` is running on this branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyWaiting;

#[derive(Default)]
struct State {
    pending: VecDeque<Arrival>,
    waiting: bool,
    /// Between a dispatched `turn/start` and its `turn/end`.
    open: bool,
}

pub struct Wait {
    state: Mutex<State>,
    arrived: Notify,
}

impl Wait {
    fn rebuild(&self, events: &[Arc<AgentEvent>]) {
        self.state.lock().unwrap().open = events
            .iter()
            .rev()
            .find_map(|event| match event.kind {
                EventKind::TurnStart(_) => Some(true),
                EventKind::TurnEnd(_) => Some(false),
                _ => None,
            })
            .unwrap_or(false);
    }
    fn consume(&self, event: &AgentEvent) {
        let mut state = self.state.lock().unwrap();
        match &event.kind {
            EventKind::TurnStart(_) => state.open = true,
            EventKind::TurnEnd(_) => {
                state.open = false;
                state.pending.clear();
            }
            // Only arrivals of an open turn count.
            EventKind::InputMessage(_) if !state.open => {}
            EventKind::InputMessage(data) => {
                if let Some(task_id) = tagged(event, WAKE_TASK_TAG) {
                    state
                        .pending
                        .push_back(Arrival::BackgroundTask(task_id.to_owned()));
                } else if data.effective_mode() == InputMode::Steer {
                    state.pending.push_back(Arrival::Steer);
                } else {
                    return;
                }
                drop(state);
                self.arrived.notify_waiters();
            }
            _ => {}
        }
    }

    /// TS `takeMatchingInput`: any steer first, then a matching task.
    fn take(&self, task_id: Option<&str>) -> Option<Arrival> {
        let mut state = self.state.lock().unwrap();
        let index = state
            .pending
            .iter()
            .position(|arrival| *arrival == Arrival::Steer)
            .or_else(|| {
                state.pending.iter().position(|arrival| match arrival {
                    Arrival::BackgroundTask(id) => task_id.is_none_or(|task_id| task_id == id),
                    Arrival::Steer => false,
                })
            })?;
        state.pending.remove(index)
    }

    /// Waits for the next matching arrival, the timeout or `cancel`.
    pub async fn wait(
        &self,
        task_id: Option<&str>,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<WaitResult, AlreadyWaiting> {
        {
            let mut state = self.state.lock().unwrap();
            if state.waiting {
                return Err(AlreadyWaiting);
            }
            state.waiting = true;
        }
        struct Release<'a>(&'a Mutex<State>);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.lock().unwrap().waiting = false;
            }
        }
        let _release = Release(&self.state);
        let started = Instant::now();
        let waited = || started.elapsed().as_millis() as u64;
        if cancel.is_cancelled() {
            return Ok(WaitResult::Interrupted {
                waited_ms: waited(),
            });
        }
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            let arrived = self.arrived.notified();
            tokio::pin!(arrived);
            arrived.as_mut().enable();
            if let Some(arrival) = self.take(task_id) {
                return Ok(match arrival {
                    Arrival::Steer => WaitResult::UserSteer {
                        waited_ms: waited(),
                    },
                    Arrival::BackgroundTask(id) => WaitResult::Notification {
                        task_ids: vec![id],
                        waited_ms: waited(),
                    },
                });
            }
            tokio::select! {
                _ = &mut arrived => {}
                _ = &mut deadline => return Ok(WaitResult::Timeout { waited_ms: waited() }),
                _ = cancel.cancelled() => {
                    return Ok(WaitResult::Interrupted { waited_ms: waited() });
                }
            }
        }
    }
}

fn wait_definition() -> ToolDefinition {
    let mut definition = definition(
        WAIT_TOOL,
        "Wait until a background task (like exec or agent) finishes, the user steers, or the timeout elapses. Pass a taskId to wait for one specific task, or omit it to wake on any.",
        json!({
            "type": "object",
            "properties": {
                "taskId": {
                    "type": "string",
                    "description": "Optional background task id to wait for. Omit to wait for any background task."
                },
                "timeoutMs": {
                    "type": "number",
                    "description": "Positive integer wait timeout in milliseconds. Default: 300000."
                }
            }
        }),
    );
    definition.source = ToolSource::Extension;
    definition.risk = Some("session".into());
    definition
}

struct WaitTool(Arc<TreeCommands>);
impl WaitTool {
    async fn run(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> Result<ToolResult, HostError> {
        let task_id = match call.input.get("taskId") {
            None => None,
            Some(Value::String(id)) => Some(id.clone()),
            Some(_) => return Ok(ToolResult::error("wait taskId must be a string.")),
        };
        let timeout_ms = match call.input.get("timeoutMs") {
            None => DEFAULT_TIMEOUT_MS,
            Some(value) => match value.as_f64() {
                Some(ms) if ms.fract() == 0.0 && ms > 0.0 && ms <= DEFAULT_TIMEOUT_MS as f64 => {
                    ms as u64
                }
                Some(ms) if ms.fract() == 0.0 && ms > DEFAULT_TIMEOUT_MS as f64 => {
                    return Ok(ToolResult::error(format!(
                        "wait timeoutMs must be less than or equal to {DEFAULT_TIMEOUT_MS}."
                    )));
                }
                _ => {
                    return Ok(ToolResult::error(
                        "wait timeoutMs must be a positive integer.",
                    ));
                }
            },
        };
        let wait = self
            .0
            .get(&context.branch_id)
            .and_then(|branch| branch.service::<Wait>())
            .map_err(|error| match error {
                AgentError::Reloading(_) => HostError::Cancelled(CAPABILITY_RELOADING.into()),
                error => HostError::failed(error.to_string()),
            })?;
        let Ok(result) = wait
            .wait(
                task_id.as_deref(),
                Duration::from_millis(timeout_ms),
                cancel,
            )
            .await
        else {
            return Ok(ToolResult::error(
                "wait is already running in this session.",
            ));
        };
        let (content, structured) = match &result {
            WaitResult::Notification {
                task_ids,
                waited_ms,
            } => (
                format!("wait ended: notification ({})", task_ids.join(", ")),
                json!({"reason": "notification", "taskIds": task_ids, "waitedMs": waited_ms}),
            ),
            WaitResult::UserSteer { waited_ms } => (
                "wait ended: user steer".into(),
                json!({"reason": "user_steer", "waitedMs": waited_ms}),
            ),
            WaitResult::Interrupted { waited_ms } => (
                "wait ended: interrupted".into(),
                json!({"reason": "interrupted", "waitedMs": waited_ms}),
            ),
            WaitResult::Timeout { waited_ms } => (
                "wait ended: timeout".into(),
                json!({"reason": "timeout", "waitedMs": waited_ms}),
            ),
        };
        let Value::Object(structured) = structured else {
            unreachable!("literal object")
        };
        Ok(ToolResult {
            structured_content: Some(structured),
            ..ToolResult::text(content)
        })
    }
}
impl ToolHandler for WaitTool {
    fn execute(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(self.run(call, context, cancel))
    }
}

pub struct WaitPlugin;
impl TypedPlugin for WaitPlugin {
    type Deps = (Keyed<TreeCommands>, Keyed<ToolRegistry>);
    fn name(&self) -> &str {
        "Wait"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(tree), Keyed(tools)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            tools.register(
                ctx,
                ToolContribution {
                    definition: wait_definition(),
                    handler: Arc::new(WaitTool(tree)),
                },
            )?;
            Ok(Effect::Done)
        })
    }
}
pub struct WaitBranchPlugin {
    wait: DepKey<Wait>,
}
impl TypedPlugin for WaitBranchPlugin {
    type Deps = (Keyed<Dispatcher>,);
    fn name(&self) -> &str {
        "WaitBranch"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(dispatcher),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let wait = Arc::new(Wait {
                state: Mutex::new(State::default()),
                arrived: Notify::new(),
            });
            // A rebuild restores only whether a turn is open; arrivals
            // before a reload are not replayed.
            let consumer = wait.clone();
            dispatcher
                .follow(
                    ctx,
                    "Wait",
                    &["input/message", "turn/start", "turn/end"],
                    move |delivery| {
                        match delivery {
                            Delivery::Replay(events) => consumer.rebuild(&events),
                            Delivery::Live(event) => consumer.consume(&event),
                        }
                        Box::pin(async { Ok(()) })
                    },
                )
                .await?;
            ctx.provide_as(self.wait.key().clone(), wait)?;
            Ok(Effect::Done)
        })
    }
}
