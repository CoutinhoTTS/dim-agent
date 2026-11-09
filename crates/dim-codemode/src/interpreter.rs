use crate::{config::CodeModeConfig, host::NestedToolHost};
use dim_agent_capabilities::{
    host::{HostError, ToolExecutionContext},
    tool_executor::NestedGate,
};
use dim_agent_types::{JsonObject, ToolCall, ToolEffect, ToolResult};
use rquickjs::{AsyncContext, AsyncRuntime, Function, Object, Promise, prelude::*};
use serde_json::json;
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::Semaphore,
    time::{Instant, timeout_at},
};
use tokio_util::sync::CancellationToken;

const CODEMODE_TOOL: &str = "codemode";

type EffectLog = Arc<Mutex<Vec<(u64, Vec<ToolEffect>)>>>;

/// Runs one program in a fresh QuickJS context. The context receives only
/// `tools.list`, `tools.describe` and `tools.call`; every child call goes
/// through the branch's `ToolExecutor` with its pinned binding and Approval.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_program(
    executor: Arc<dyn NestedToolHost>,
    config: Arc<CodeModeConfig>,
    outer_call_id: String,
    code: String,
    timeout_ms: Option<u64>,
    max_calls: Option<u64>,
    context: ToolExecutionContext,
    cancel: CancellationToken,
) -> Result<ToolResult, HostError> {
    if code.len() > config.max_code_bytes {
        return Ok(ToolResult::error(format!(
            "Code Mode program exceeds the {} byte code limit",
            config.max_code_bytes
        )));
    }
    let timeout = Duration::from_millis(
        timeout_ms
            .unwrap_or(config.max_runtime_ms)
            .min(config.max_runtime_ms),
    );
    let max_calls = max_calls.unwrap_or(config.max_calls).min(config.max_calls);
    let deadline = Instant::now() + timeout;

    let definitions = match executor.definitions(&context.request_id) {
        Ok(definitions) => definitions
            .into_iter()
            .filter(|tool| tool.name != CODEMODE_TOOL)
            .collect::<Vec<_>>(),
        Err(message) => return Ok(ToolResult::error(message)),
    };
    let allowed = Arc::new(
        definitions
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<HashSet<_>>(),
    );
    let definitions = Arc::new(definitions);
    let calls = Arc::new(AtomicU64::new(0));
    let parallel = Arc::new(Semaphore::new(config.max_parallel_calls.max(1)));
    let effects = Arc::new(Mutex::new(Vec::<(u64, Vec<ToolEffect>)>::new()));
    let child_cancel = cancel.child_token();
    let gate = match executor.open(outer_call_id, context.clone(), child_cancel.clone()) {
        Ok(gate) => gate,
        Err(error) => return Ok(ToolResult::error(error.to_string())),
    };

    let runtime = AsyncRuntime::new()
        .map_err(|error| HostError::failed(format!("QuickJS runtime: {error}")))?;
    runtime.set_memory_limit(config.max_memory_bytes).await;
    let interrupted = child_cancel.clone();
    runtime
        .set_interrupt_handler(Some(Box::new(move || {
            instant_passed(deadline) || interrupted.is_cancelled()
        })))
        .await;

    let context_handle = AsyncContext::full(&runtime)
        .await
        .map_err(|error| HostError::failed(format!("QuickJS context: {error}")))?;
    let output_limit = config.max_output_bytes;
    let cancel_after = cancel.clone();
    let effects_after = effects.clone();
    let gate_after = gate.clone();
    let child_cancel_after = child_cancel.clone();
    let run = context_handle.async_with(async move |ctx| {
        let globals = ctx.globals();
        let tools = Object::new(ctx.clone())?;

        let list_definitions = definitions.clone();
        let list = Function::new(ctx.clone(), move || -> rquickjs::Result<String> {
            let list = list_definitions
                .iter()
                .map(|tool| json!({"name": tool.name, "description": tool.description}))
                .collect::<Vec<_>>();
            Ok(serde_json::to_string(&list).expect("tool summaries serialize"))
        })?;
        tools.set("list", list)?;

        let describe_definitions = definitions.clone();
        let describe = Function::new(
            ctx.clone(),
            move |name: String| -> rquickjs::Result<Option<String>> {
                Ok(describe_definitions
                    .iter()
                    .find(|tool| tool.name == name)
                    .and_then(|tool| serde_json::to_string(tool).ok()))
            },
        )?;
        tools.set("describe", describe)?;

        let call_scope = CallScope {
            gate: gate.clone(),
            cancel: child_cancel.clone(),
            count: calls.clone(),
            parallel: parallel.clone(),
            effects: effects.clone(),
            allowed: allowed.clone(),
            deadline,
            max_calls,
            single_limit: config.max_single_result_bytes,
        };
        let call = Function::new(
            ctx.clone(),
            Async(move |name: String, arguments: String| {
                call_nested_tool(call_scope.clone(), name, arguments)
            }),
        )?;
        tools.set("call", call)?;
        globals.set("__codemodeTools", tools)?;

        let program = format!(
            r#"
(async () => {{
  const __host = globalThis.__codemodeTools;
  globalThis.tools = {{
    list: () => JSON.parse(__host.list()),
    describe: (name) => {{
      const value = __host.describe(String(name));
      return value === null ? undefined : JSON.parse(value);
    }},
    call: async (name, args) => {{
      const value = JSON.parse(await __host.call(String(name), JSON.stringify(args ?? {{}})));
      if (value.isError) {{
        const message = value.message || value.content || "tool failed";
        const error = new Error(typeof message === "string" ? message : JSON.stringify(message));
        error.code = value.error || "tool_error";
        throw error;
      }}
      return value;
    }},
  }};
  const __value = await (async () => {{
{code}
  }})();
  return JSON.stringify(__value === undefined ? null : __value);
}})()
"#
        );
        let promise: Promise = ctx.eval(program)?;
        let output: String = promise.into_future().await?;
        Ok::<_, rquickjs::Error>(output)
    });
    let result = tokio::select! {
        biased;
        _ = cancel_after.cancelled() => None,
        result = timeout_at(deadline, run) => result.ok(),
    };
    child_cancel_after.cancel();
    gate_after.close().await;

    match result {
        Some(Ok(output)) => {
            let result = if output.len() > output_limit {
                ToolResult::error(format!(
                    "Code Mode output exceeds the {output_limit} byte limit"
                ))
            } else {
                ToolResult::text(output)
            };
            Ok(with_effects(result, &effects_after))
        }
        Some(Err(error)) => {
            if cancel_after.is_cancelled() {
                Ok(with_effects(
                    ToolResult::error("Code Mode program cancelled"),
                    &effects_after,
                ))
            } else if instant_passed(deadline) {
                Ok(with_effects(
                    ToolResult::error("Code Mode program timed out"),
                    &effects_after,
                ))
            } else {
                Ok(with_effects(
                    ToolResult::error(format!("Code Mode program failed: {error}")),
                    &effects_after,
                ))
            }
        }
        None => {
            if cancel_after.is_cancelled() {
                Ok(with_effects(
                    ToolResult::error("Code Mode program cancelled"),
                    &effects_after,
                ))
            } else {
                Ok(with_effects(
                    ToolResult::error("Code Mode program timed out"),
                    &effects_after,
                ))
            }
        }
    }
}

#[derive(Clone)]
struct CallScope {
    gate: Arc<dyn NestedGate>,
    cancel: CancellationToken,
    count: Arc<AtomicU64>,
    parallel: Arc<Semaphore>,
    effects: EffectLog,
    allowed: Arc<HashSet<String>>,
    deadline: Instant,
    max_calls: u64,
    single_limit: usize,
}

async fn call_nested_tool(
    scope: CallScope,
    name: String,
    arguments: String,
) -> rquickjs::Result<String> {
    let index = scope.count.fetch_add(1, Ordering::SeqCst);
    if index >= scope.max_calls {
        return Ok(error_json(
            "call_limit",
            "the program exceeded its nested-call budget",
        ));
    }
    if !scope.allowed.contains(&name) {
        return Ok(error_json(
            "not_allowed",
            "the tool is not in this request's Code Mode tool list",
        ));
    }
    let permit = tokio::select! {
        biased;
        _ = scope.cancel.cancelled() => {
            return Ok(error_json("cancelled", "the program stopped before the call started"));
        }
        _ = tokio::time::sleep_until(scope.deadline) => {
            return Ok(error_json("timeout", "the call budget expired before the call started"));
        }
        permit = scope.parallel.acquire() => match permit {
            Ok(permit) => permit,
            Err(_) => {
                return Ok(error_json("cancelled", "the program stopped before the call started"));
            }
        },
    };
    let input: JsonObject = match serde_json::from_str(&arguments) {
        Ok(input) => input,
        Err(error) => {
            drop(permit);
            return Ok(error_json("invalid_arguments", &error.to_string()));
        }
    };
    let call = ToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        input,
    };
    let result = scope.gate.invoke(call).await;
    drop(permit);
    match result {
        Ok(result) => {
            if let Some(child_effects) = result.effects.clone() {
                scope.effects.lock().unwrap().push((index, child_effects));
            }
            let text = serde_json::to_string(&result)
                .unwrap_or_else(|_| error_json("invalid_result", "tool result did not serialize"));
            if text.len() > scope.single_limit {
                Ok(error_json(
                    "result_too_large",
                    "the tool result exceeded the Code Mode per-call limit",
                ))
            } else {
                Ok(text)
            }
        }
        Err(error) => Ok(error_json("tool_error", &error.to_string())),
    }
}

fn with_effects(
    mut result: ToolResult,
    effects: &Mutex<Vec<(u64, Vec<ToolEffect>)>>,
) -> ToolResult {
    let mut collected = std::mem::take(&mut *effects.lock().unwrap());
    collected.sort_by_key(|(sequence, _)| *sequence);
    let effects = collected
        .into_iter()
        .flat_map(|(_, effects)| effects)
        .collect::<Vec<_>>();
    if !effects.is_empty() {
        result.effects = Some(effects);
    }
    result
}

fn instant_passed(deadline: Instant) -> bool {
    Instant::now() >= deadline
}

fn error_json(code: &str, message: &str) -> String {
    json!({"isError": true, "error": code, "message": message}).to_string()
}
