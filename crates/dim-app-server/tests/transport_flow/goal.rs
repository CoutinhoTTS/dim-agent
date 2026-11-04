//! Goal (docs/design/plugins/Goal.md) as a TypeScript plugin, one row: its
//! Session part writes change inputs by branch id, and its branch part folds
//! the branch's events and decides continuation. Pursuit (Pursuit.md) adds the
//! evaluator judge: a completion request creates an evaluator branch whose
//! conclusion the working branch decides on.
use super::reload::last_system;
use super::*;
use dim_agent_capabilities::{
    adapters::DefaultPrepareModelPlugin,
    history::HistoryRenderer,
    host::{
        CompactionStrategyHost, ContextModelHost, HostError, IdentityContext, ModelChunk,
        ModelConnection, ModelDriver, ModelHost, ModelStopReason, ModelStream, PrepareContextHost,
    },
};
use dim_agent_runtime::testing::{branch_events, chain_events};
use dim_agent_types::{BranchCreateData, BranchSource, MessageContent, ModelRequest};
use dim_app_server::session_typescript::SessionTypeScriptConfig;
use std::path::{Path, PathBuf};

fn goal_rows() -> SessionTypeScriptConfig {
    let mut config = session_typescript::fixture();
    config.rows = vec![session_typescript::manifest_row("goal")];
    config
}
fn product_home() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let home = PathBuf::from("/tmp").join(format!("dg-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    home
}
async fn select_model(client: &mut Client) {
    let models = [
        json!({"modelId":"fixture","displayName":"fixture","status":"active",
        "capabilities":dim_agent_types::CatalogCapabilities::with_limits(128_000, 16_384)}),
    ];
    assert_ok(&client.rpc("provider.create", json!({"providerId":"018ec73f-0000-7000-8000-000000000005","displayName":"Fixture","connection":{"driverKind":"openai-compatible","baseUrl":"http://localhost/v1","auth":{"type":"anonymous"}},"models":models,"enabled":true})).await);
    assert_ok(
        &client
            .rpc(
                "provider.select",
                json!({"providerId":"018ec73f-0000-7000-8000-000000000005","modelId":"fixture"}),
            )
            .await,
    );
}
async fn set_limit(client: &mut Client, limit: u32) {
    let read = super::settings_methods::registered(client, "goal").await;
    assert_ok(&read);
    assert_ok(
        &client
            .rpc(
                "settings.update",
                json!({"source":"goal","scope":"user","expectedRevision":read["result"]["revision"],
                    "document":{"maxContinuations":limit}}),
            )
            .await,
    );
}
async fn new_session(client: &mut Client) -> String {
    let mut params = create(json!(["goal_update", "wait"]));
    params["permissionPolicy"]["allowedTools"] = json!(["goal_update", "wait"]);
    let created = client.rpc("session.create", params).await;
    assert_ok(&created);
    created["result"]["sessionId"].as_str().unwrap().to_owned()
}
async fn goal(client: &mut Client, method: &str, params: Value) -> Value {
    client.rpc(&format!("session.goal.{method}"), params).await
}
/// Goal's blocks on `branch`, in order: each input's first `<dim-goal>`
/// marker line, and the input's sequence.
async fn goal_writes(session: &dim_agent_runtime::Session, branch: &str) -> Vec<(u64, String)> {
    let Ok(events) = branch_events(session, branch, ..).await else {
        return Vec::new();
    };
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::InputMessage(data) => data.blocks.first().and_then(|block| {
                let dim_agent_types::InputBlock::Raw { text, .. } = block;
                text.starts_with("<dim-goal ")
                    .then(|| (event.sequence, text.lines().next().unwrap().to_owned()))
            }),
            _ => None,
        })
        .collect()
}
/// Waits until a Goal block on `branch` contains `needle`.
async fn wait_for(session: &dim_agent_runtime::Session, branch: &str, needle: &str) {
    let found = tokio::time::timeout(Duration::from_secs(15), async {
        while !goal_writes(session, branch)
            .await
            .iter()
            .any(|(_, line)| line.contains(needle))
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        found.is_ok(),
        "no Goal block with {needle}: {:#?}",
        branch_events(session, branch, ..).await
    );
}
async fn idle(client: &mut Client, session: &str, branch: &str) {
    assert_ok(
        &client
            .rpc(
                "session.idle",
                json!({"sessionId":session,"branchId":branch}),
            )
            .await,
    );
}
/// The value a change block carries after its `label` line, decoded by the
/// block encoding of Goal A.5.
fn block_value(text: &str, label: &str) -> String {
    let start = text.find(&format!("\n{label}\n")).unwrap() + label.len() + 2;
    let end = text.rfind("\n\n").unwrap();
    text[start..end]
        .replace("&quot;", "\"")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
}
/// The full text of the last Goal block on `branch` whose marker contains `needle`.
async fn goal_block(session: &dim_agent_runtime::Session, branch: &str, needle: &str) -> String {
    branch_events(session, branch, ..)
        .await
        .unwrap()
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::InputMessage(data) => data.blocks.first().and_then(|block| {
                let dim_agent_types::InputBlock::Raw { text, .. } = block;
                (text.starts_with("<dim-goal ") && text.lines().next().unwrap().contains(needle))
                    .then(|| text.clone())
            }),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no Goal block with {needle}"))
}

/// The raw blocks of the last Goal input on `branch` whose marker line
/// contains `needle`, in order.
async fn goal_blocks(
    session: &dim_agent_runtime::Session,
    branch: &str,
    needle: &str,
) -> Vec<String> {
    let Ok(events) = branch_events(session, branch, ..).await else {
        panic!("no Goal input with {needle}")
    };
    events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::InputMessage(data) => data.blocks.first().and_then(|block| {
                let dim_agent_types::InputBlock::Raw { text, .. } = block;
                (text.starts_with("<dim-goal ") && text.lines().next().unwrap().contains(needle))
                    .then(|| {
                        data.blocks
                            .iter()
                            .map(|block| {
                                let dim_agent_types::InputBlock::Raw { text, .. } = block;
                                text.clone()
                            })
                            .collect()
                    })
            }),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no Goal input with {needle}"))
}

/// The last model request on `branch`, as the model received it.
async fn last_request(session: &dim_agent_runtime::Session, branch: &str) -> Arc<ModelRequest> {
    chain_events(session, branch, ..)
        .await
        .unwrap()
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(data.request.clone()),
            _ => None,
        })
        .expect("a model call")
}

/// Every tool result text on `branch`, with whether it was an error.
async fn tool_texts(session: &dim_agent_runtime::Session, branch: &str) -> Vec<(String, bool)> {
    branch_events(session, branch, ..)
        .await
        .unwrap()
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ToolResult(data) => Some(&data.results),
            _ => None,
        })
        .flatten()
        .map(|result| {
            let text = match &result.result.content {
                dim_agent_types::ToolContent::Text(text) => text.clone(),
                dim_agent_types::ToolContent::Blocks(blocks) => {
                    serde_json::to_string(blocks).unwrap()
                }
            };
            (text, result.result.is_error == Some(true))
        })
        .collect()
}

/// Waits until a tool result with exactly `text` is on `branch`.
async fn wait_tool_text(session: &dim_agent_runtime::Session, branch: &str, text: &str) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if tool_texts(session, branch)
                .await
                .iter()
                .any(|(got, _)| got == text)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no tool result {text:?} on {branch}"));
}

/// The `goal_update` tool's declaration, as the model receives it.
async fn assert_goal_tool(session: &dim_agent_runtime::Session, branch: &str) {
    let request = last_request(session, branch).await;
    let tools = request.tools.as_deref().expect("a request with tools");
    let tool = tools
        .iter()
        .find(|tool| tool.name == "goal_update")
        .unwrap_or_else(|| panic!("goal_update is offered: {tools:#?}"));
    assert_eq!(
        tool.description,
        "Report that the current goal is completed or blocked."
    );
    assert_eq!(
        serde_json::to_value(&tool.input_schema).unwrap(),
        json!({
            "type": "object",
            "properties": {
                "status": {"type": "string", "enum": ["completed", "blocked"]},
                "summary": {"type": "string", "minLength": 1, "maxLength": 2000},
            },
            "required": ["status", "summary"],
            "additionalProperties": false,
        }),
        "the goal_update schema"
    );
    assert_eq!(tool.source, dim_agent_types::ToolSource::Extension);
    assert_eq!(tool.risk.as_deref(), Some("session"));
}

/// A.5's encoding, for expected values written as raw text.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The two blocks a `set` writes.
fn set_blocks(branch: &str, objective: &str) -> (String, String) {
    (
        format!(
            "<dim-goal kind=\"goal\" op=\"set\" branch=\"{branch}\">\nObjective:\n{}\n\n\
             This is the user's standing goal for branch {branch}. If you are on another line of this Session, the goal belongs to branch {branch}: treat it as background, do not continue it, and do not call goal_update for it.\n\
             </dim-goal>",
            escape(objective)
        ),
        active_status(branch),
    )
}

/// [`set_blocks`] under the evaluator judge.
fn set_blocks_judged(branch: &str, objective: &str) -> (String, String) {
    let (objective_block, status) = set_blocks(branch, objective);
    (
        objective_block.replace("op=\"set\" ", "op=\"set\" judge=\"evaluator\" "),
        status,
    )
}

fn active_status(branch: &str) -> String {
    format!(
        "<dim-goal kind=\"status\" status=\"active\" branch=\"{branch}\">\n\
         The goal for branch {branch} is active. Keep working toward it across turns. When it is fully achieved, call goal_update with status \"completed\". If you cannot make progress without the user, call goal_update with status \"blocked\" and explain why.\n\
         </dim-goal>"
    )
}

fn edit_block(branch: &str, objective: &str) -> String {
    format!(
        "<dim-goal kind=\"goal\" op=\"edit\" branch=\"{branch}\">\nObjective:\n{}\n\n\
         The user set the objective of the goal for branch {branch}. This does not change its status.\n\
         </dim-goal>",
        escape(objective)
    )
}

fn user_pause_block(branch: &str) -> String {
    format!(
        "<dim-goal kind=\"goal\" op=\"pause\" reason=\"user\" branch=\"{branch}\">\n\
         The user set the goal status for branch {branch} to paused. Do not continue working toward a goal for branch {branch} on your own; respond only to the user's latest input.\n\
         </dim-goal>"
    )
}

fn resume_block(branch: &str) -> String {
    format!(
        "<dim-goal kind=\"goal\" op=\"resume\" branch=\"{branch}\">\n\
         The user set the goal status for branch {branch} to active. If branch {branch} has a goal objective, keep working toward it across turns; when it is completed or blocked, call goal_update.\n\
         </dim-goal>"
    )
}

/// The two blocks a `clear` writes; `after` is the opening it answers.
fn clear_blocks(branch: &str, after: Option<u64>) -> (String, String) {
    let (marker, body) = match after {
        Some(after) => (
            format!(
                "kind=\"goal\" op=\"clear\" reason=\"new_branch\" after=\"{after}\" branch=\"{branch}\""
            ),
            format!(
                "Any goal above belongs to another branch, not to branch {branch}. Disregard it; branch {branch} has no goal."
            ),
        ),
        None => (
            format!("kind=\"goal\" op=\"clear\" branch=\"{branch}\""),
            "The previous goal was cleared by the user. Disregard it.".into(),
        ),
    };
    (
        format!("<dim-goal {marker}>\n{body}\n</dim-goal>"),
        format!(
            "<dim-goal kind=\"status\" status=\"cleared\" branch=\"{branch}\">\n\
             The goal for branch {branch} has no status.\n\
             </dim-goal>"
        ),
    )
}

fn report_block(branch: &str, status: &str, summary: &str) -> String {
    let label = if status == "completed" {
        "Result:"
    } else {
        "Blocker:"
    };
    format!(
        "<dim-goal kind=\"goal\" op=\"{status}\" branch=\"{branch}\">\n{label}\n{}\n\n\
         goal_update set the goal status for branch {branch} to {status}. Do not continue working toward a goal for branch {branch} on your own.\n\
         </dim-goal>",
        escape(summary)
    )
}

fn continuation_block(branch: &str, count: usize, limit: u64, after: u64) -> String {
    format!(
        "<dim-goal kind=\"continuation\" count=\"{count}\" limit=\"{limit}\" after=\"{after}\" branch=\"{branch}\">\n\
         Continue working toward the active goal for branch {branch}. If it is completed or blocked, call goal_update. If you are on another line of this Session, the goal belongs to branch {branch}: do not call goal_update for it.\n\
         </dim-goal>"
    )
}

fn pause_block(branch: &str, reason: &str, phrase: &str, after: u64) -> String {
    format!(
        "<dim-goal kind=\"goal\" op=\"pause\" reason=\"{reason}\" after=\"{after}\" branch=\"{branch}\">\n\
         The goal is paused ({phrase}). Do not continue working toward it on your own; respond only to the user's latest input.\n\
         </dim-goal>"
    )
}

fn evaluate_again_block(branch: &str, after: u64) -> String {
    format!(
        "<dim-goal kind=\"evaluate-again\" after=\"{after}\" branch=\"{branch}\">\n\
         Your reply did not end with a conclusion. End your reply with exactly one of DONE:, CONTINUE: or BLOCKED: as described above.\n\
         </dim-goal>"
    )
}

fn conclusion_block(
    branch: &str,
    evaluator: &str,
    evaluation: &str,
    result: &str,
    text: &str,
) -> String {
    format!(
        "<dim-goal kind=\"report\" result=\"{result}\" evaluation=\"{evaluation}\" evaluator=\"{evaluator}\" branch=\"{branch}\">\n\
         Evaluator report:\n{}\n\
         </dim-goal>",
        escape(text)
    )
}

fn evaluated_block(branch: &str, op: &str, evaluation: &str, text: &str) -> String {
    let (label, sentence) = if op == "completed" {
        (
            "Result:",
            format!(
                "An evaluator verified that the goal for branch {branch} is completed. Do not continue working toward it on your own."
            ),
        )
    } else {
        (
            "Blocker:",
            format!(
                "An evaluator found that the goal for branch {branch} cannot proceed without the user. Do not continue working toward it on your own."
            ),
        )
    };
    format!(
        "<dim-goal kind=\"goal\" op=\"{op}\" by=\"evaluator\" evaluation=\"{evaluation}\" branch=\"{branch}\">\n{label}\n{}\n\n{sentence}\n\
         </dim-goal>",
        escape(text)
    )
}

fn evaluated_continuation_block(
    branch: &str,
    count: usize,
    limit: u64,
    after: u64,
    evaluation: &str,
) -> String {
    format!(
        "<dim-goal kind=\"continuation\" count=\"{count}\" limit=\"{limit}\" after=\"{after}\" evaluation=\"{evaluation}\" branch=\"{branch}\">\n\
         An evaluator checked your report that the goal for branch {branch} is completed and found it is not achieved yet. Its report above lists items that are at least still missing; the list may be incomplete. Assess yourself how far the goal still is, then continue working toward it. When you think it is achieved, call goal_update with status \"completed\" again.\n\
         </dim-goal>"
    )
}

/// The evaluator prompt (Pursuit §5): the first Queue input of its branch.
fn evaluator_prompt(branch: &str, evaluation: &str, objective: &str, summary: &str) -> String {
    format!(
        "<dim-goal kind=\"evaluate\" branch=\"{}\" evaluation=\"{evaluation}\">\n\
         You are an evaluator. Branch {branch} reported that its goal is completed. Only you decide whether it is. Do not continue the work, do not change files or settings, and do not call goal_update.\n\
         \n\
         Objective:\n{}\n\
         \n\
         Reported result:\n{}\n\
         \n\
         Verify the facts yourself by reading files and running checks; do not rely on the report or on earlier messages. End your reply with exactly one of:\n\
         DONE: <the verified result>\n\
         CONTINUE: followed by the items that are at least still missing, one per line, each with its evidence\n\
         BLOCKED: <what only the user can provide>",
        escape(branch),
        escape(objective),
        escape(summary)
    )
}

/// Text with `<`, `&`, quotes, an entity-like run and a blank line.
const SPECIAL: &str = "ship <it> & \"now\"\n\nthen &amp; done";

fn cleanup(home: &Path) {
    let _ = std::fs::remove_dir_all(home);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_goal_continues_to_the_limit_and_stops_on_completion() {
    let home = product_home();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.timeline_store(Arc::new(dim_agent_runtime::MemoryTimelineStore::default()));
    let (server, agent) = launch_goal(&home, builder).await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 2).await;

    // Limit (Appendix D 1, 2): set drives a turn; each normal end writes a
    // continuation until the third decision pauses with `limit`.
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    let set = goal(
        &mut client,
        "set",
        json!({"sessionId":id,"objective":"ship <it>"}),
    )
    .await;
    assert_ok(&set);
    assert!(set["result"]["sequence"].is_u64(), "{set}");
    wait_for(&session, "main", r#"reason="limit""#).await;
    idle(&mut client, &id, "main").await;
    let writes: Vec<String> = goal_writes(&session, "main")
        .await
        .into_iter()
        .map(|(_, line)| line)
        .collect();
    assert_eq!(writes.len(), 4, "{writes:#?}");
    assert!(writes[0].contains(r#"op="set""#), "{writes:#?}");
    assert!(
        writes[1].contains(r#"kind="continuation" count="1" limit="2""#),
        "{writes:#?}"
    );
    assert!(
        writes[2].contains(r#"kind="continuation" count="2" limit="2""#),
        "{writes:#?}"
    );
    assert!(
        writes[3].contains(r#"op="pause" reason="limit""#),
        "{writes:#?}"
    );
    // Every model-facing text, byte for byte (C2). The three turns are the
    // set's, and the two continuations'.
    let ends = turn_ends(&session, "main").await;
    assert_eq!(ends.len(), 3, "three turns: {ends:?}");
    let (set_objective, set_status) = set_blocks("main", "ship <it>");
    assert_eq!(
        goal_blocks(&session, "main", r#"op="set""#).await,
        vec![set_objective, set_status],
        "the set input's two blocks"
    );
    assert_eq!(
        goal_block(&session, "main", r#"count="1""#).await,
        continuation_block("main", 1, 2, ends[0])
    );
    assert_eq!(
        goal_block(&session, "main", r#"count="2""#).await,
        continuation_block("main", 2, 2, ends[1])
    );
    assert_eq!(
        goal_block(&session, "main", r#"reason="limit""#).await,
        pause_block(
            "main",
            "limit",
            "the automatic continuation limit was reached",
            ends[2]
        )
    );
    assert_goal_tool(&session, "main").await;

    // A child branch cut after the goal starts with none: Goal retires the
    // inherited keys once (§3.4, Appendix D 7).
    let child = session
        .branch(
            "main".into(),
            session.sequence("main").unwrap(),
            dim_agent_runtime::BranchOptions {
                prompt: "child task".into(),
                source: dim_agent_types::BranchSource::Host {
                    operation_id: "goal-child".into(),
                },
            },
        )
        .await
        .unwrap();
    wait_for(&session, &child, r#"reason="new_branch""#).await;
    idle(&mut client, &id, &child).await;
    let child_writes = goal_writes(&session, &child).await;
    assert_eq!(child_writes.len(), 1, "{child_writes:#?}");
    assert_eq!(goal_writes(&session, "main").await.len(), 4);
    let opening = branch_events(&session, &child, ..)
        .await
        .unwrap()
        .iter()
        .find(|event| matches!(event.kind, EventKind::BranchStart(_)))
        .expect("the child's branch/start")
        .sequence;
    let (clear_objective, clear_status) = clear_blocks(&child, Some(opening));
    assert_eq!(
        goal_blocks(&session, &child, r#"reason="new_branch""#).await,
        vec![clear_objective, clear_status],
        "the new-branch clear's two blocks"
    );

    // Completion (Appendix D 1): the model's goal_update records `completed`
    // and no continuation follows it.
    set_limit(&mut client, 5).await;
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":"finish"}),
        )
        .await,
    );
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":id,"branchId":"main",
                    "text":format!("tool:goal_update {}", json!({"status":"completed","summary":SPECIAL}))}),
            )
            .await,
    );
    wait_for(&session, "main", r#"op="completed""#).await;
    idle(&mut client, &id, "main").await;
    let (finish_set, finish_status) = set_blocks("main", "finish");
    assert_eq!(
        goal_blocks(&session, "main", r#"op="set""#).await,
        vec![finish_set, finish_status],
        "the second session's set"
    );
    assert_eq!(
        goal_block(&session, "main", r#"op="completed""#).await,
        report_block("main", "completed", SPECIAL),
        "the completed report"
    );
    wait_tool_text(&session, "main", "goal_update recorded: completed.").await;
    let completed_block = goal_block(&session, "main", r#"op="completed""#).await;
    assert!(!completed_block.contains("<it>"), "{completed_block}");
    assert_eq!(block_value(&completed_block, "Result:"), SPECIAL);
    let writes = goal_writes(&session, "main").await;
    let (completed, _) = writes
        .iter()
        .find(|(_, line)| line.contains(r#"op="completed""#))
        .unwrap()
        .clone();
    assert!(
        writes
            .iter()
            .all(|(sequence, line)| *sequence <= completed || !line.contains("continuation")),
        "{writes:#?}"
    );
    let results: Vec<String> = branch_events(&session, "main", ..)
        .await
        .unwrap()
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ToolResult(data) => Some(serde_json::to_string(data).unwrap()),
            _ => None,
        })
        .collect();
    assert!(
        results
            .iter()
            .any(|result| result.contains("goal_update recorded: completed.")),
        "{results:#?}"
    );

    for (params, error) in [
        (json!({"sessionId":id,"objective":""}), "invalid_params"),
        (
            json!({"sessionId":id,"branchId":"missing","objective":"x"}),
            "session_invalid_input",
        ),
    ] {
        assert_eq!(
            goal(&mut client, "set", params).await["error"]["data"]["appError"],
            error
        );
    }
    for method in ["pause", "resume", "clear"] {
        assert_ok(&goal(&mut client, method, json!({"sessionId":id})).await);
    }
    wait_for(&session, "main", r#"op="clear""#).await;
    idle(&mut client, &id, "main").await;
    assert_eq!(
        goal_block(&session, "main", r#"op="pause" reason="user""#).await,
        user_pause_block("main")
    );
    assert_eq!(
        goal_block(&session, "main", r#"op="resume""#).await,
        resume_block("main")
    );
    let (clear_objective, clear_status) = clear_blocks("main", None);
    assert_eq!(
        goal_blocks(&session, "main", r#"op="clear""#).await,
        vec![clear_objective, clear_status],
        "the user's clear"
    );

    // The tool's rejection text, and a `blocked` report.
    submit(
        &mut client,
        &id,
        r#"tool:goal_update {"status":"nope","summary":"x"}"#,
    )
    .await;
    wait_tool_text(
        &session,
        "main",
        "goal_update needs status \"completed\" or \"blocked\" and a summary of 1 to 2000 characters.",
    )
    .await;
    submit_report(&mut client, &id, "blocked", "needs credentials").await;
    wait_for(&session, "main", r#"op="blocked""#).await;
    idle(&mut client, &id, "main").await;
    assert_eq!(
        goal_block(&session, "main", r#"op="blocked""#).await,
        report_block("main", "blocked", "needs credentials")
    );
    wait_tool_text(&session, "main", "goal_update recorded: blocked.").await;
    let after: Vec<String> = goal_writes(&session, "main")
        .await
        .into_iter()
        .filter(|(sequence, _)| *sequence > completed)
        .map(|(_, line)| line)
        .collect();
    let at = |op: &str| {
        after
            .iter()
            .position(|line| line.contains(&format!(r#"op="{op}""#)))
            .unwrap_or_else(|| panic!("no {op}: {after:#?}"))
    };
    assert!(
        at("pause") < at("resume") && at("resume") < at("clear"),
        "{after:#?}"
    );
    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);
}

async fn launch_goal(
    home: &Path,
    builder: AgentBuilder,
) -> (RunningServer, dim_agent_runtime::Agent) {
    launch_goal_with(
        home,
        builder,
        Arc::new(dim_agent_capabilities::adapters::EchoModel),
    )
    .await
}

async fn launch_goal_with(
    home: &Path,
    mut builder: AgentBuilder,
    model: Arc<dyn ModelDriver>,
) -> (RunningServer, dim_agent_runtime::Agent) {
    defaults::install(&mut builder);
    builder
        .session_value(Arc::new(HistoryRenderer::default()))
        .process_value(Arc::new(ModelHost(model)))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_value(Arc::new(CompactionStrategyHost(None)))
        .require_feature(
            Feature::new("PrepareModelHost").session("PrepareModelHost", |names| {
                Ok(Box::new(DefaultPrepareModelPlugin::new(names)?))
            }),
        );
    // Goal stays an ordinary optional row: its tool is adopted whenever it
    // becomes active (#241), with no dependency forcing it first (#223).
    dim_agent_capabilities::defaults::tool_host(&mut builder, Arc::new(MockTools));
    let server = serve(
        builder,
        ServerConfig {
            unix_path: Some(home.join("server.sock")),
            product_home: Some(home.to_owned()),
            target_home: Some(home.join("rust")),
            session_typescript: Some(goal_rows()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let agent = server.agent().unwrap();
    (server, agent)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_goal_restore_refolds_without_repeating_decisions() {
    let home = product_home();
    let (server, agent) = launch_goal(&home, AgentBuilder::new(AgentConfig::default())).await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 1).await;
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":SPECIAL}),
        )
        .await,
    );
    wait_for(&session, "main", r#"reason="limit""#).await;
    idle(&mut client, &id, "main").await;
    assert_eq!(
        block_value(
            &goal_block(&session, "main", r#"op="set""#).await,
            "Objective:"
        ),
        SPECIAL
    );
    let edited = format!("{SPECIAL} (edited)");
    assert_ok(
        &goal(
            &mut client,
            "edit",
            json!({"sessionId":id,"objective":edited}),
        )
        .await,
    );
    wait_for(&session, "main", r#"op="edit""#).await;
    idle(&mut client, &id, "main").await;
    assert_eq!(
        goal_block(&session, "main", r#"op="edit""#).await,
        edit_block("main", &edited)
    );
    let before = goal_writes(&session, "main").await;
    assert_eq!(before.len(), 4, "{before:#?}");
    drop(client);
    server.shutdown();
    server.join().await.unwrap();

    let (server, agent) = launch_goal(&home, AgentBuilder::new(AgentConfig::default())).await;
    let mut client = Client::new_version(&server, false, 3).await;
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":id,"branchId":"main","text":"hello again"}),
            )
            .await,
    );
    idle(&mut client, &id, "main").await;
    let session = agent.get_session(&id).unwrap();
    assert_eq!(goal_writes(&session, "main").await, before);
    assert_eq!(
        block_value(
            &goal_block(&session, "main", r#"op="edit""#).await,
            "Objective:"
        ),
        edited
    );
    assert_ok(&goal(&mut client, "resume", json!({"sessionId":id})).await);
    wait_for(&session, "main", r#"kind="continuation" count="1""#).await;
    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);
}

/// Waits until a local event on `branch` matches. A branch that is still
/// loading has no published events yet.
async fn wait_event(
    session: &dim_agent_runtime::Session,
    branch: &str,
    matches: impl Fn(&EventKind) -> bool,
) {
    for _ in 0..750 {
        let Ok(events) = branch_events(session, branch, ..).await else {
            tokio::time::sleep(Duration::from_millis(20)).await;
            continue;
        };
        if events.iter().any(|event| matches(&event.kind)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let events = branch_events(session, branch, ..).await.unwrap_or_default();
    panic!(
        "event did not arrive: {:#?}",
        events
            .iter()
            .map(|event| serde_json::to_string(&event.kind).unwrap())
            .collect::<Vec<_>>()
    );
}
/// Sequences of the `turn/end` events on `branch`.
async fn turn_ends(session: &dim_agent_runtime::Session, branch: &str) -> Vec<u64> {
    branch_events(session, branch, ..)
        .await
        .unwrap()
        .iter()
        .filter(|event| matches!(event.kind, EventKind::TurnEnd(_)))
        .map(|event| event.sequence)
        .collect()
}
async fn submit(client: &mut Client, session: &str, text: &str) {
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":session,"branchId":"main","text":text}),
            )
            .await,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_goal_pending_input_and_interrupts() {
    let home = product_home();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.timeline_store(Arc::new(dim_agent_runtime::MemoryTimelineStore::default()));
    let (server, agent) = launch_goal(&home, builder).await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 1).await;

    // Appendix D 4: the goal is set while a turn runs, and the operator
    // queues more input. The turn's end finds Queue input pending, so it
    // decides nothing; the turn that consumes them decides.
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    submit(&mut client, &id, r#"tool:wait {"millis":1500}"#).await;
    wait_event(&session, "main", |kind| {
        matches!(kind, EventKind::ToolCall(_))
    })
    .await;
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":"ship"}),
        )
        .await,
    );
    submit(&mut client, &id, "and the changelog").await;
    wait_for(&session, "main", r#"reason="limit""#).await;
    idle(&mut client, &id, "main").await;
    let ends = turn_ends(&session, "main").await;
    let writes = goal_writes(&session, "main").await;
    let decided: Vec<&String> = writes
        .iter()
        .map(|(_, line)| line)
        .filter(|line| line.contains("after="))
        .collect();
    assert!(
        decided
            .iter()
            .all(|line| !line.contains(&format!(r#"after="{}""#, ends[0]))),
        "{ends:?} {writes:#?}"
    );
    assert!(
        decided
            .iter()
            .any(|line| line.contains(r#"kind="continuation" count="1""#)),
        "{writes:#?}"
    );
    let (continuation, _) = writes
        .iter()
        .find(|(_, line)| line.contains(r#"count="1""#))
        .expect("the first continuation")
        .clone();
    let deciding = ends
        .iter()
        .rev()
        .find(|end| **end < continuation)
        .copied()
        .expect("the deciding turn");
    assert_eq!(
        goal_block(&session, "main", r#"count="1""#).await,
        continuation_block("main", 1, 1, deciding)
    );

    // Appendix D 3: an interrupt after the goal starts pauses it, and no
    // continuation follows.
    set_limit(&mut client, 5).await;
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    submit(&mut client, &id, r#"tool:wait {"millis":1500}"#).await;
    wait_event(&session, "main", |kind| {
        matches!(kind, EventKind::ToolCall(_))
    })
    .await;
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":"ship"}),
        )
        .await,
    );
    wait_for(&session, "main", r#"op="set""#).await;
    assert_ok(
        &client
            .rpc("session.stop", json!({"sessionId":id,"branchId":"main"}))
            .await,
    );
    wait_for(&session, "main", r#"reason="interrupted""#).await;
    idle(&mut client, &id, "main").await;
    let interrupt = branch_events(&session, "main", ..)
        .await
        .unwrap()
        .iter()
        .rev()
        .find(|event| matches!(event.kind, EventKind::StopInterrupt(_)))
        .expect("the interrupt")
        .sequence;
    assert_eq!(
        goal_block(&session, "main", r#"reason="interrupted""#).await,
        pause_block(
            "main",
            "interrupted",
            "the user interrupted the turn",
            interrupt
        )
    );
    let writes = goal_writes(&session, "main").await;
    assert!(
        writes
            .iter()
            .all(|(_, line)| !line.contains("continuation")),
        "{writes:#?}"
    );
    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_goal_settings_resume_unload_and_fork() {
    let home = product_home();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.timeline_store(Arc::new(dim_agent_runtime::MemoryTimelineStore::default()));
    let (server, agent) = launch_goal(&home, builder).await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 1).await;
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":"ship"}),
        )
        .await,
    );
    wait_for(&session, "main", r#"reason="limit""#).await;
    idle(&mut client, &id, "main").await;

    // Appendix D 11: completed, then resume reopens the goal with the count
    // restarted. Appendix D 8: read the committed limit at the next decision,
    // without reloading Goal or waiting for a settings.changed callback.
    submit(
        &mut client,
        &id,
        &format!(
            "tool:goal_update {}",
            json!({"status":"completed","summary":"done"})
        ),
    )
    .await;
    wait_for(&session, "main", r#"op="completed""#).await;
    idle(&mut client, &id, "main").await;
    set_limit(&mut client, 3).await;
    assert_ok(&goal(&mut client, "resume", json!({"sessionId":id})).await);
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let writes = goal_writes(&session, "main").await;
            let limits = writes
                .iter()
                .filter(|(_, line)| line.contains(r#"reason="limit""#))
                .count();
            if limits == 2 {
                assert!(
                    writes
                        .iter()
                        .any(|(_, line)| line.contains(r#"count="3" limit="3""#)),
                    "{writes:#?}"
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the resumed goal pauses at the new limit");
    idle(&mut client, &id, "main").await;

    // Appendix D 8: an unloaded Session is restored by a goal method, which
    // writes.
    assert!(session.unload_if_idle().await.unwrap());
    drop(session);
    assert_ok(&goal(&mut client, "pause", json!({"sessionId":id})).await);
    let session = agent.get_session(&id).unwrap();
    wait_for(&session, "main", r#"op="pause" reason="user""#).await;
    idle(&mut client, &id, "main").await;

    // Appendix D 7: a fork writes `session/forked` after the copied facts.
    // The forked `main`'s goal starts empty: one start `clear` after that
    // fact, and no continuation.
    let fork = client
        .rpc(
            "session.fork",
            json!({"sessionId":id,"at":session.sequence("main").unwrap()}),
        )
        .await;
    assert_ok(&fork);
    let fork_id = fork["result"]["sessionId"].as_str().unwrap().to_owned();
    let forked = agent.get_session(&fork_id).unwrap();
    wait_for(&forked, "main", r#"reason="new_branch""#).await;
    idle(&mut client, &fork_id, "main").await;
    let events = branch_events(&forked, "main", ..).await.unwrap();
    let opening = events
        .iter()
        .find(|event| matches!(&event.kind, EventKind::SessionForked(_)))
        .expect("the fork's fact")
        .sequence;
    let writes = goal_writes(&forked, "main").await;
    let own: Vec<&String> = writes
        .iter()
        .filter(|(sequence, _)| *sequence > opening)
        .map(|(_, line)| line)
        .collect();
    assert_eq!(own.len(), 1, "{writes:#?}");
    assert!(
        own[0].contains(&format!(r#"reason="new_branch" after="{opening}""#)),
        "{own:#?}"
    );
    let (fork_clear, fork_status) = clear_blocks("main", Some(opening));
    assert_eq!(
        goal_blocks(&forked, "main", r#"reason="new_branch""#).await,
        vec![fork_clear, fork_status],
        "the fork's new-branch clear"
    );
    assert!(
        goal_writes(&session, "main")
            .await
            .iter()
            .all(|(_, line)| !line.contains("new_branch"))
    );

    server.shutdown();
    server.join().await.unwrap();

    cleanup(&home);

    // Appendix D 8: an assembly without Goal has no goal methods.
    let home = product_home();
    let server = launch(
        Arc::new(MockTools),
        ServerConfig {
            product_home: Some(home.clone()),
            target_home: Some(home.join("rust")),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    let created = client.rpc("session.create", create(json!([]))).await;
    assert_ok(&created);
    let id = created["result"]["sessionId"].as_str().unwrap().to_owned();
    assert_eq!(
        goal(&mut client, "set", json!({"sessionId":id,"objective":"x"})).await["error"]["data"]["appError"],
        "unknown_method"
    );
    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);
}

/// Whether the request is the evaluator's: its prompt's first line is Goal's
/// evaluate marker (Pursuit §5).
fn is_evaluator(request: &ModelRequest) -> bool {
    request.messages.iter().any(|message| {
        matches!(&message.content, MessageContent::Text(text)
            if text.contains("<dim-goal kind=\"evaluate\""))
    })
}

/// Answers the evaluator's prompt with a DONE conclusion; echoes otherwise.
struct EvaluatingModel;
impl ModelDriver for EvaluatingModel {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        if !is_evaluator(&request) {
            return dim_agent_capabilities::adapters::EchoModel.invoke(request, connection, cancel);
        }
        Box::pin(async move {
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "evaluation".into(),
                }),
                Ok(ModelChunk::Text(
                    "Read the tests and ran them.\nDONE: all tests pass".into(),
                )),
                Ok(ModelChunk::TextEnd),
                Ok(ModelChunk::MessageStop {
                    message_id: "evaluation".into(),
                    reason: ModelStopReason::EndTurn,
                }),
            ])) as ModelStream)
        })
    }
}

/// Holds the evaluator's turn in a long tool wait; echoes otherwise.
struct WaitingModel;
impl ModelDriver for WaitingModel {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        if !is_evaluator(&request) {
            return dim_agent_capabilities::adapters::EchoModel.invoke(request, connection, cancel);
        }
        Box::pin(async move {
            let call = ToolCall {
                id: "evaluation-wait".into(),
                name: "wait".into(),
                input: json!({"millis": 60000}).as_object().unwrap().clone(),
            };
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "evaluation".into(),
                }),
                Ok(ModelChunk::ToolStart {
                    id: call.id.clone(),
                    name: call.name.clone(),
                }),
                Ok(ModelChunk::ToolUse(call)),
                Ok(ModelChunk::MessageStop {
                    message_id: "evaluation".into(),
                    reason: ModelStopReason::ToolUse,
                }),
            ])) as ModelStream)
        })
    }
}

/// The `branch/create` events on `branch` in order, with their sequences.
async fn branch_creates(
    session: &dim_agent_runtime::Session,
    branch: &str,
) -> Vec<(u64, BranchCreateData)> {
    branch_events(session, branch, ..)
        .await
        .unwrap()
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::BranchCreate(data) => Some((event.sequence, data.clone())),
            _ => None,
        })
        .collect()
}

/// The user text of the first input on `branch`. A branch that is still being
/// built has no published events yet.
async fn first_user_text(session: &dim_agent_runtime::Session, branch: &str) -> String {
    for _ in 0..750 {
        let Ok(events) = branch_events(session, branch, ..).await else {
            tokio::time::sleep(Duration::from_millis(20)).await;
            continue;
        };
        for event in events {
            if let EventKind::InputMessage(data) = &event.kind
                && let Some(user) = &data.user
                && let Some(dim_agent_types::ContentBlock::Text { text }) = user.content.first()
            {
                return text.clone();
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no user input on {branch}")
}

/// Submits the worker's `goal_update`.
async fn submit_report(client: &mut Client, session: &str, status: &str, summary: &str) {
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":session,"branchId":"main",
                    "text":format!("tool:goal_update {}", json!({"status":status,"summary":summary}))}),
            )
            .await,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_goal_on_a_line_behaves_as_on_main() {
    let home = product_home();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.timeline_store(Arc::new(dim_agent_runtime::MemoryTimelineStore::default()));
    let (server, agent) = launch_goal(&home, builder).await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 1000).await;
    let mut params = create(json!(["goal_update", "wait", "branch"]));
    params["permissionPolicy"]["allowedTools"] = json!(["goal_update", "wait", "branch"]);
    let created = client.rpc("session.create", params).await;
    assert_ok(&created);
    let id = created["result"]["sessionId"].as_str().unwrap().to_owned();
    let session = agent.get_session(&id).unwrap();
    let at = session.sequence("main").unwrap();
    let created = client
        .rpc(
            "session.branch_create",
            json!({"sessionId":id,"parentBranchId":"main","at":at,"prompt":"line work"}),
        )
        .await;
    assert_ok(&created);
    let child = created["result"]["branch"]["branchId"]
        .as_str()
        .unwrap()
        .to_owned();
    idle(&mut client, &id, &child).await;
    let opening = branch_events(&session, &child, ..)
        .await
        .unwrap()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::InputMessage(data) => data.blocks.first().map(|block| {
                let dim_agent_types::InputBlock::Raw { text, .. } = block;
                (text.clone(), data.user.clone())
            }),
            _ => None,
        })
        .expect("the line's opening input");
    let marker = format!(r#"<dim-branch-line branch="{child}" parent="main" source="host">"#);
    assert!(opening.0.starts_with(&marker), "{}", opening.0);
    assert_eq!(
        first_user_text(&session, &child).await,
        "line work",
        "the creator's prompt stays the opening input's words"
    );
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"branchId":child,"objective":"line objective"}),
        )
        .await,
    );
    wait_for(&session, &child, r#"op="set""#).await;
    let (line_set, line_status) = set_blocks(&child, "line objective");
    assert_eq!(
        goal_blocks(&session, &child, r#"op="set""#).await,
        vec![line_set, line_status],
        "the line's set"
    );
    wait_for(&session, &child, r#"count="1""#).await;
    idle(&mut client, &id, &child).await;
    let operator = session
        .submit(
            "keep going".into(),
            dim_agent_types::InputMode::Queue,
            &child,
        )
        .await
        .unwrap()
        .sequence;
    let (reset, expected) = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let writes = goal_writes(&session, &child).await;
            let reset = writes.iter().find(|(_, line)| {
                line.contains(r#"kind="continuation""#)
                    && line
                        .split("after=\"")
                        .nth(1)
                        .and_then(|rest| rest.split('"').next())
                        .and_then(|value| value.parse::<u64>().ok())
                        .is_some_and(|after| after > operator)
            });
            if let Some((sequence, line)) = reset {
                let raced = writes
                    .iter()
                    .filter(|(at, other)| {
                        other.contains(r#"kind="continuation""#) && *at > operator && at < sequence
                    })
                    .count();
                return (line.clone(), 1 + raced);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("a continuation after the operator input");
    assert!(
        reset.contains(&format!(r#"count="{expected}""#)),
        "the continuation after the operator input counts {expected}: {reset}"
    );
    let after = reset
        .split("after=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .and_then(|value| value.parse::<u64>().ok())
        .expect("the continuation's after");
    assert_eq!(
        goal_block(&session, &child, &format!(r#"count="{expected}""#)).await,
        continuation_block(&child, expected, 1000, after),
        "the line's continuation"
    );
    assert!(goal_writes(&session, "main").await.is_empty());
    assert_ok(
        &goal(
            &mut client,
            "pause",
            json!({"sessionId":id,"branchId":child}),
        )
        .await,
    );
    wait_for(&session, &child, r#"op="pause""#).await;
    idle(&mut client, &id, &child).await;
    assert_eq!(
        goal_block(&session, &child, r#"op="pause""#).await,
        user_pause_block(&child)
    );
    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_goal_evaluator_judge_creates_an_evaluator_and_completes_on_done() {
    let home = product_home();
    let (server, agent) = launch_goal_with(
        &home,
        AgentBuilder::new(AgentConfig::default()),
        Arc::new(EvaluatingModel),
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 5).await;
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":"make the tests pass","judge":"evaluator"}),
        )
        .await,
    );
    wait_for(&session, "main", r#"op="set""#).await;
    let (judged_set, judged_status) = set_blocks_judged("main", "make the tests pass");
    assert_eq!(
        goal_blocks(&session, "main", r#"op="set""#).await,
        vec![judged_set, judged_status],
        "the judged set"
    );
    submit_report(&mut client, &id, "completed", "all tests pass").await;
    wait_event(&session, "main", |kind| {
        matches!(kind, EventKind::BranchCreate(_))
    })
    .await;
    idle(&mut client, &id, "main").await;
    let creates = branch_creates(&session, "main").await;
    assert_eq!(creates.len(), 1, "{creates:#?}");
    let (created, creation) = &creates[0];
    let evaluation = turn_ends(&session, "main")
        .await
        .last()
        .copied()
        .expect("a turn end");
    assert_eq!(
        creation.source,
        BranchSource::Host {
            operation_id: format!("goal:evaluate:{evaluation}")
        },
        "{creation:#?}"
    );
    let writes = goal_writes(&session, "main").await;
    let (request, _) = writes
        .iter()
        .find(|(_, line)| line.contains(r#"op="completed""#) && !line.contains("by="))
        .unwrap_or_else(|| panic!("the worker's request: {writes:#?}"));
    assert!(
        *request < *created,
        "the request precedes the creation: {writes:#?}"
    );
    assert!(
        writes
            .iter()
            .all(|(_, line)| !line.contains("continuation")),
        "{writes:#?}"
    );
    let child = &creation.branch;
    let prompt = first_user_text(&session, child).await;
    assert_eq!(
        prompt,
        evaluator_prompt(
            "main",
            &evaluation.to_string(),
            "make the tests pass",
            "all tests pass"
        ),
        "the evaluator's prompt"
    );
    wait_for(&session, "main", r#"by="evaluator""#).await;
    idle(&mut client, &id, "main").await;
    let done = goal_block(&session, "main", r#"by="evaluator""#).await;
    assert_eq!(
        done,
        evaluated_block(
            "main",
            "completed",
            &evaluation.to_string(),
            "all tests pass"
        ),
        "the evaluator's completion"
    );
    // The conclusion it decides on is one input of one block, and that
    // block's marker is the report of this evaluation and evaluator
    // (Pursuit §5).
    let blocks = branch_events(&session, "main", ..)
        .await
        .unwrap()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::InputMessage(data) => data.blocks.first().and_then(|block| {
                let dim_agent_types::InputBlock::Raw { text, .. } = block;
                text.starts_with(r#"<dim-goal kind="report""#)
                    .then_some(data.blocks.len())
            }),
            _ => None,
        })
        .expect("the evaluator's conclusion input");
    assert_eq!(blocks, 1, "the conclusion input carries the report alone");
    let conclusion = branch_events(&session, "main", ..)
        .await
        .unwrap()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::InputMessage(data) => data.blocks.first().and_then(|block| {
                let dim_agent_types::InputBlock::Raw { text, .. } = block;
                text.starts_with(r#"<dim-goal kind="report""#)
                    .then(|| text.clone())
            }),
            _ => None,
        })
        .expect("the evaluator's conclusion input");
    assert_eq!(
        conclusion,
        conclusion_block(
            "main",
            child,
            &evaluation.to_string(),
            "done",
            "DONE: all tests pass"
        ),
        "the conclusion input"
    );
    let writes = goal_writes(&session, "main").await;
    assert!(
        writes
            .iter()
            .filter(|(_, line)| line.contains(r#"by="evaluator""#))
            .count()
            == 1,
        "{writes:#?}"
    );
    wait_event(
        &session,
        "main",
        |kind| matches!(kind, EventKind::BranchEnded(data) if data.branch == *child),
    )
    .await;
    idle(&mut client, &id, "main").await;
    let head = session.sequence(child).unwrap();
    assert_eq!(session.ended_head(child), Some(head));
    let line_events = session
        .session_service::<dim_agent_capabilities::timeline_tree::TreeCommands>()
        .unwrap()
        .page(child.to_owned(), 0, 256)
        .await
        .unwrap();
    assert_eq!(
        line_events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::TurnEnd(_)))
            .count(),
        1,
        "one evaluator turn"
    );
    let endings: Vec<Value> = branch_events(&session, "main", ..)
        .await
        .unwrap()
        .iter()
        .filter(
            |event| matches!(&event.kind, EventKind::BranchEnded(data) if data.branch == *child),
        )
        .map(|event| serde_json::to_value(&event.kind).unwrap())
        .collect();
    assert_eq!(endings.len(), 1, "{endings:#?}");
    assert_eq!(
        endings[0]["data"]["source"],
        json!({"kind": "host", "operation_id": format!("goal:evaluate:{evaluation}")}),
        "{endings:#?}"
    );
    assert_eq!(endings[0]["data"]["branch"], *child, "{endings:#?}");
    let line = client
        .rpc(
            "session.history",
            json!({"sessionId":id,"branchId":child,"after":0,"limit":256}),
        )
        .await;
    assert_ok(&line);
    assert_eq!(line["result"]["done"], true, "{line:#?}");
    assert!(
        line["result"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["type"] == "branch"
                && event["name"] == "end"
                && event["data"]["source"]["operation_id"]
                    == format!("goal:evaluate:{evaluation}")),
        "{line:#?}"
    );
    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_goal_evaluator_restart_writes_no_second_result() {
    let home = product_home();
    let (server, agent) = launch_goal_with(
        &home,
        AgentBuilder::new(AgentConfig::default()),
        Arc::new(WaitingModel),
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 5).await;
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":"ship it","judge":"evaluator"}),
        )
        .await,
    );
    submit_report(&mut client, &id, "completed", "done").await;
    wait_event(&session, "main", |kind| {
        matches!(kind, EventKind::BranchCreate(_))
    })
    .await;
    let creates = branch_creates(&session, "main").await;
    let child = creates[0].1.branch.clone();
    wait_event(&session, &child, |kind| {
        matches!(kind, EventKind::ToolCall(_))
    })
    .await;
    drop(client);
    server.shutdown();
    server.join().await.unwrap();

    let (server, agent) = launch_goal_with(
        &home,
        AgentBuilder::new(AgentConfig::default()),
        Arc::new(EvaluatingModel),
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    submit(&mut client, &id, "back again").await;
    idle(&mut client, &id, "main").await;
    let session = agent.get_session(&id).unwrap();
    assert_eq!(branch_creates(&session, "main").await.len(), 1);
    let writes = goal_writes(&session, "main").await;
    assert!(
        writes
            .iter()
            .all(|(_, line)| !line.contains(r#"by="evaluator""#)),
        "a replayed turn end reports nothing: {writes:#?}"
    );
    assert!(
        writes
            .iter()
            .all(|(_, line)| !line.contains(r#"reason="evaluation_failed""#)),
        "{writes:#?}"
    );
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":id,"branchId":child,"text":"carry on"}),
            )
            .await,
    );
    wait_for(&session, "main", r#"by="evaluator""#).await;
    idle(&mut client, &id, "main").await;
    let writes = goal_writes(&session, "main").await;
    assert!(
        writes
            .iter()
            .filter(|(_, line)| line.contains(r#"by="evaluator""#))
            .count()
            == 1,
        "{writes:#?}"
    );
    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);
}

/// Answers the evaluator's first turn without a conclusion and its second
/// with CONTINUE; echoes otherwise.
#[derive(Default)]
struct RevisingModel {
    evaluator_turns: std::sync::atomic::AtomicUsize,
}
impl ModelDriver for RevisingModel {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        if !is_evaluator(&request) {
            return dim_agent_capabilities::adapters::EchoModel.invoke(request, connection, cancel);
        }
        let turn = self
            .evaluator_turns
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let text = if turn == 0 {
            "I read the tests.".to_owned()
        } else {
            "Still missing.\nCONTINUE:\n- tests still fail".to_owned()
        };
        Box::pin(async move {
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "evaluation".into(),
                }),
                Ok(ModelChunk::Text(text)),
                Ok(ModelChunk::TextEnd),
                Ok(ModelChunk::MessageStop {
                    message_id: "evaluation".into(),
                    reason: ModelStopReason::EndTurn,
                }),
            ])) as ModelStream)
        })
    }
}

/// Answers the evaluator's turn with BLOCKED; echoes otherwise.
struct BlockedModel;
impl ModelDriver for BlockedModel {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        if !is_evaluator(&request) {
            return dim_agent_capabilities::adapters::EchoModel.invoke(request, connection, cancel);
        }
        Box::pin(async move {
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "evaluation".into(),
                }),
                Ok(ModelChunk::Text(
                    "Blocked on credentials.\nBLOCKED: need credentials".into(),
                )),
                Ok(ModelChunk::TextEnd),
                Ok(ModelChunk::MessageStop {
                    message_id: "evaluation".into(),
                    reason: ModelStopReason::EndTurn,
                }),
            ])) as ModelStream)
        })
    }
}

/// Fails the evaluator's turn; echoes every other request.
struct FailingEvaluator;
impl ModelDriver for FailingEvaluator {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        if !is_evaluator(&request) {
            return dim_agent_capabilities::adapters::EchoModel.invoke(request, connection, cancel);
        }
        Box::pin(async move { Err(HostError::failed("fixture evaluator failure")) })
    }
}

/// Fails a turn whose last operator text is `fail`; echoes otherwise.
struct FailingModel;
impl ModelDriver for FailingModel {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        let fails = request.messages.iter().rev().find_map(|message| {
            (message.role == dim_agent_types::Role::User).then(|| match &message.content {
                MessageContent::Text(text) => text.contains("fail"),
                _ => false,
            })
        });
        if fails != Some(true) {
            return dim_agent_capabilities::adapters::EchoModel.invoke(request, connection, cancel);
        }
        Box::pin(async move { Err(HostError::failed("fixture failure")) })
    }
}

/// The evaluation a `branch/create` on `branch` names.
async fn evaluation_of(session: &dim_agent_runtime::Session, branch: &str) -> String {
    let creates = branch_creates(session, branch).await;
    let creation = &creates.first().expect("the evaluator's creation").1;
    let BranchSource::Host { operation_id } = &creation.source else {
        panic!("a host creation: {creation:#?}")
    };
    operation_id
        .strip_prefix("goal:evaluate:")
        .expect("Goal's operation id")
        .to_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_goal_evaluator_asks_again_and_continues() {
    let home = product_home();
    let (server, agent) = launch_goal_with(
        &home,
        AgentBuilder::new(AgentConfig::default()),
        Arc::new(RevisingModel::default()),
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 5).await;
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":"make the tests pass","judge":"evaluator"}),
        )
        .await,
    );
    submit_report(&mut client, &id, "completed", "all tests pass").await;
    wait_event(&session, "main", |kind| {
        matches!(kind, EventKind::BranchCreate(_))
    })
    .await;
    let evaluation = evaluation_of(&session, "main").await;
    let child = branch_creates(&session, "main").await[0].1.branch.clone();

    // The first evaluator turn ends without a conclusion: Goal asks again on
    // the evaluator's own line.
    wait_for(&session, &child, r#"kind="evaluate-again""#).await;
    let first_end = turn_ends(&session, &child).await[0];
    assert_eq!(
        goal_block(&session, &child, r#"kind="evaluate-again""#).await,
        evaluate_again_block(&child, first_end),
        "the evaluator's follow-up"
    );

    // The second ends with CONTINUE: its report decides the evaluation, and
    // the working branch continues.
    let needle = format!(r#"evaluation="{evaluation}""#);
    wait_for(&session, "main", &needle).await;
    idle(&mut client, &id, "main").await;
    assert_eq!(
        goal_block(&session, "main", r#"kind="report""#).await,
        conclusion_block(
            "main",
            &child,
            &evaluation,
            "continue",
            "CONTINUE:\n- tests still fail"
        ),
        "the CONTINUE report"
    );
    let continuation = goal_block(&session, "main", &needle).await;
    let count = continuation
        .split("count=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .and_then(|value| value.parse::<usize>().ok())
        .expect("the continuation's count");
    let after = continuation
        .split("after=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .and_then(|value| value.parse::<u64>().ok())
        .expect("the continuation's after");
    assert_eq!(
        continuation,
        evaluated_continuation_block("main", count, 5, after, &evaluation),
        "the evaluator's continuation"
    );
    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_goal_evaluator_blocks_or_fails_the_evaluation() {
    // An evaluator that reports BLOCKED completes the goal as blocked.
    let home = product_home();
    let (server, agent) = launch_goal_with(
        &home,
        AgentBuilder::new(AgentConfig::default()),
        Arc::new(BlockedModel),
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 5).await;
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":"ship it","judge":"evaluator"}),
        )
        .await,
    );
    submit_report(&mut client, &id, "completed", "done").await;
    wait_event(&session, "main", |kind| {
        matches!(kind, EventKind::BranchCreate(_))
    })
    .await;
    let evaluation = evaluation_of(&session, "main").await;
    let child = branch_creates(&session, "main").await[0].1.branch.clone();
    wait_for(&session, "main", r#"by="evaluator""#).await;
    idle(&mut client, &id, "main").await;
    assert_eq!(
        goal_block(&session, "main", r#"by="evaluator""#).await,
        evaluated_block("main", "blocked", &evaluation, "need credentials"),
        "the evaluator's block"
    );
    assert_eq!(
        goal_block(&session, "main", r#"kind="report""#).await,
        conclusion_block(
            "main",
            &child,
            &evaluation,
            "blocked",
            "BLOCKED: need credentials"
        ),
        "the BLOCKED report"
    );
    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);

    // An evaluator whose turn fails pauses the goal with `evaluation_failed`.
    let home = product_home();
    let (server, agent) = launch_goal_with(
        &home,
        AgentBuilder::new(AgentConfig::default()),
        Arc::new(FailingEvaluator),
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 5).await;
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":"ship it","judge":"evaluator"}),
        )
        .await,
    );
    submit_report(&mut client, &id, "completed", "done").await;
    wait_event(&session, "main", |kind| {
        matches!(kind, EventKind::BranchCreate(_))
    })
    .await;
    let evaluation = evaluation_of(&session, "main").await;
    let child = branch_creates(&session, "main").await[0].1.branch.clone();
    wait_for(&session, "main", r#"reason="evaluation_failed""#).await;
    idle(&mut client, &id, "main").await;
    let after = evaluation.parse::<u64>().expect("a sequence");
    assert_eq!(
        goal_block(&session, "main", r#"reason="evaluation_failed""#).await,
        pause_block(
            "main",
            "evaluation_failed",
            "the evaluator could not check the result",
            after
        ),
        "the failed evaluation"
    );
    assert_eq!(
        goal_block(&session, "main", r#"kind="report""#).await,
        conclusion_block(
            "main",
            &child,
            &evaluation,
            "failed",
            "the evaluator's turn did not complete"
        ),
        "the evaluator's failure report"
    );
    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_goal_turn_failure_pauses_the_goal() {
    let home = product_home();
    let (server, agent) = launch_goal_with(
        &home,
        AgentBuilder::new(AgentConfig::default()),
        Arc::new(FailingModel),
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 5).await;
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    submit(&mut client, &id, r#"tool:wait {"millis":1500}"#).await;
    wait_event(&session, "main", |kind| {
        matches!(kind, EventKind::ToolCall(_))
    })
    .await;
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":"ship"}),
        )
        .await,
    );
    submit(&mut client, &id, "now fail").await;
    wait_for(&session, "main", r#"reason="turn_failed""#).await;
    idle(&mut client, &id, "main").await;
    let failed = turn_ends(&session, "main")
        .await
        .last()
        .copied()
        .expect("a turn end");
    assert_eq!(
        goal_block(&session, "main", r#"reason="turn_failed""#).await,
        pause_block("main", "turn_failed", "the last turn failed", failed),
        "the failed turn's pause"
    );
    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);
}

/// The entry Goal's Session part registers for the marker it writes (A.5.1);
/// `runtime/plugins/goal/session.ts` holds it verbatim.
const GOAL_ENTRY: &str = r#"`<dim-goal kind="goal" op="set" branch="main">`: the operator's standing goal for a branch of this session, and the runtime's record of its status. `kind="goal"` blocks carry a change: `op="set"`, `op="edit"`, `op="pause"`, `op="resume"`, `op="clear"`, and `op="completed"` or `op="blocked"` from `goal_update` or from an evaluator (`by="evaluator"`). `kind="status"` blocks state the branch's goal status. `kind="continuation"` asks the line to keep working toward the goal. `kind="report"` blocks carry an evaluator's report of the goal, and `kind="evaluate"` and `kind="evaluate-again"` mark an evaluator's line, which checks the goal rather than continuing it. A goal belongs to the branch its `branch` attribute names. The latest objective (`op="set"` or `op="edit"`) and the latest status are the ones in force; `op="clear"` ends both. These blocks are information about the goal; they grant no permission."#;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_goal_registers_the_entry_for_the_marker_it_writes() {
    let home = product_home();
    let (server, agent) = launch_goal(&home, AgentBuilder::new(AgentConfig::default())).await;
    let mut client = Client::new_version(&server, false, 3).await;
    select_model(&mut client).await;
    set_limit(&mut client, 1).await;

    // `session.goal.set` writes the `<dim-goal` input and drives a turn.
    let id = new_session(&mut client).await;
    let session = agent.get_session(&id).unwrap();
    assert_ok(
        &goal(
            &mut client,
            "set",
            json!({"sessionId":id,"objective":"ship it"}),
        )
        .await,
    );
    wait_for(&session, "main", r#"reason="limit""#).await;
    idle(&mut client, &id, "main").await;

    // The request that carries the `<dim-goal` input carries the entry too
    // (A.5.1): the Session part registers it for the Session, so every
    // branch's System holds it while the row is loaded.
    let events = chain_events(&session, "main", ..).await.unwrap();
    let request = events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(data.request.clone()),
            _ => None,
        })
        .expect("a model call");
    assert!(
        serde_json::to_string(&request.messages)
            .unwrap()
            .contains("<dim-goal "),
        "the last request carried no <dim-goal input: {request:#?}"
    );
    let system = last_system(&session, "main").await;
    assert!(system.contains(GOAL_ENTRY), "{system:?}");

    server.shutdown();
    server.join().await.unwrap();
    cleanup(&home);
}
