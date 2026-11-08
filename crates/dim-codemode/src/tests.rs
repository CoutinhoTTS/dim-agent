use crate::{CodeModeConfig, NestedToolHost, interpreter::run_program};
use dim_agent_capabilities::{
    host::{HostError, ToolExecutionContext},
    tool_registry::definition,
};
use dim_agent_types::{ToolCall, ToolContent, ToolDefinition, ToolResult, ToolSource};
use serde_json::json;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio_util::sync::CancellationToken;

struct MockHost {
    definitions: Vec<ToolDefinition>,
}

impl NestedToolHost for MockHost {
    fn definitions(&self, _: &str) -> Result<Vec<ToolDefinition>, String> {
        Ok(self.definitions.clone())
    }

    fn invoke(
        &self,
        call: ToolCall,
        _: ToolExecutionContext,
        _: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, HostError>> + Send + '_>> {
        Box::pin(async move {
            match call.name.as_str() {
                "echo" => Ok(ToolResult::text(
                    serde_json::to_string(&call.input).expect("input serializes"),
                )),
                "fail" => Ok(ToolResult::error("boom")),
                "write" => {
                    let mut result = ToolResult::text("wrote");
                    result.effects = Some(vec![dim_agent_types::ToolEffect::FileWritten {
                        path: "/tmp/codemode".into(),
                    }]);
                    Ok(result)
                }
                other => Ok(ToolResult::error(format!("unknown tool {other}"))),
            }
        })
    }
}

fn context() -> ToolExecutionContext {
    ToolExecutionContext {
        turn_id: "turn".into(),
        request_id: "request".into(),
        branch_id: "branch".into(),
    }
}

fn host() -> Arc<MockHost> {
    let mut codemode = definition("codemode", "self", json!({"type": "object"}));
    codemode.source = ToolSource::Extension;
    Arc::new(MockHost {
        definitions: vec![
            definition("echo", "echo", json!({"type": "object"})),
            definition("write", "write", json!({"type": "object"})),
            codemode,
        ],
    })
}

async fn run(code: &str, max_calls: Option<u64>) -> ToolResult {
    run_program(
        host(),
        Arc::new(CodeModeConfig::default()),
        code.into(),
        None,
        max_calls,
        context(),
        CancellationToken::new(),
    )
    .await
    .expect("program settles")
}

fn text(result: &ToolResult) -> &str {
    match &result.content {
        ToolContent::Text(text) => text,
        ToolContent::Blocks(_) => panic!("expected text"),
    }
}

#[tokio::test]
async fn calls_a_pinned_tool_and_returns_its_value() {
    let result = run(
        r#"const value = await tools.call("echo", {x: 1}); return value;"#,
        None,
    )
    .await;
    let value: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    assert_eq!(value["content"], "{\"x\":1}");
    assert!(value.get("isError").is_none());
}

#[tokio::test]
async fn lists_only_request_tools() {
    let result = run("return tools.list();", None).await;
    let value: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    let names = value
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["echo", "write"]);
}

#[tokio::test]
async fn a_tool_error_is_a_catchable_program_error() {
    let result = run(
        r#"try { await tools.call("fail", {}); } catch (error) { return {code: error.code}; }"#,
        None,
    )
    .await;
    let value: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    assert_eq!(value["code"], "tool_error");
}

#[tokio::test]
async fn the_call_budget_is_enforced() {
    let result = run(
        r#"const values = [];
            for (let i = 0; i < 2; i++) {
              try { values.push(await tools.call("echo", {i})); }
              catch (error) { values.push(error.code); }
            }
            return values;"#,
        Some(1),
    )
    .await;
    let value: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    assert_eq!(value[0]["content"], "{\"i\":0}");
    assert_eq!(value[1], "call_limit");
}

#[tokio::test]
async fn child_effects_reach_the_outer_result() {
    let result = run(r#"await tools.call("write", {}); return "ok";"#, None).await;
    let effects = result.effects.expect("effects");
    assert_eq!(
        effects,
        vec![dim_agent_types::ToolEffect::FileWritten {
            path: "/tmp/codemode".into()
        }]
    );
}

#[tokio::test]
async fn a_synchronous_loop_is_interrupted() {
    let config = CodeModeConfig {
        max_runtime_ms: 50,
        ..Default::default()
    };
    let result = run_program(
        host(),
        Arc::new(config),
        "while (true) {}".into(),
        None,
        None,
        context(),
        CancellationToken::new(),
    )
    .await
    .expect("program settles");
    assert_eq!(text(&result), "Code Mode program timed out");
}
