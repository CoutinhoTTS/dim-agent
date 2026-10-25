//! Branch permission through the public Session interfaces.
use dim_agent_capabilities::{
    approval::Approval,
    defaults,
    host::*,
    model_provider::ModelProviderPlugin,
    permission::{
        CommandRulesHost, Decision, Permission, PermissionState, PermissionUpdateError,
        ReadCommandRules,
    },
    prompt::InitialPromptConfiguration,
    tool_registry::definition,
};
use dim_agent_runtime::*;
use dim_agent_types::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// One tool per risk category; `hold` waits for the test.
#[derive(Default)]
struct RiskTools {
    calls: AtomicUsize,
    entered: Notify,
    release: Notify,
}
const TOOLS: [(&str, Option<&str>); 7] = [
    ("read", Some("read_filesystem")),
    ("write", Some("write_filesystem")),
    ("exec", Some("execute_process")),
    ("mcp", Some("network")),
    ("skill", Some("session")),
    ("plain", None),
    ("hold", Some("execute_process")),
];
impl ToolDriver for RiskTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        TOOLS
            .iter()
            .map(|(name, risk)| ToolDefinition {
                risk: risk.map(Into::into),
                ..definition(name, "Risk probe", serde_json::json!({"type":"object"}))
            })
            .collect()
    }
    fn execute(
        &self,
        call: ToolCall,
        _: ToolExecutionContext,
        _: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            if call.name == "hold" {
                self.entered.notify_one();
                self.release.notified().await;
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ToolResult::text(format!("{} ran", call.name)))
        })
    }
}

/// The user's command rules as the test sets them; `None` is an unreadable
/// source.
struct TestRules(Mutex<Option<Vec<CommandRule>>>);
impl TestRules {
    fn new(rules: Vec<CommandRule>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Some(rules))))
    }
    fn set(&self, rules: Option<Vec<CommandRule>>) {
        *self.0.lock().unwrap() = rules;
    }
}
impl ReadCommandRules for TestRules {
    fn rules(&self, _: CancellationToken) -> HostFuture<'_, Vec<CommandRule>> {
        let rules = self.0.lock().unwrap().clone();
        Box::pin(async move { rules.ok_or_else(|| HostError::failed("invalid rules file")) })
    }
}
fn rule(command: &str, matching: CommandMatch, decision: CommandRuleDecision) -> CommandRule {
    CommandRule {
        command: command.into(),
        matching,
        decision,
    }
}

fn agent(tools: Arc<RiskTools>) -> Agent {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.timeline_store(Arc::new(MemoryTimelineStore::default()));
    defaults::install(&mut builder);
    defaults::mock_model_hosts(&mut builder);
    defaults::tool_host(&mut builder, tools);
    builder.build()
}
/// The default profile with `rules` bound in place of its empty host.
fn ruled_agent(tools: Arc<RiskTools>, rules: Arc<dyn ReadCommandRules>) -> Agent {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder
        .timeline_store(Arc::new(MemoryTimelineStore::default()))
        .session_value(Arc::new(PromptHost(Arc::new(EmptyPromptHost))))
        .session_value(Arc::new(InitialPromptConfiguration(None)))
        .session_value(Arc::new(CommandRulesHost(rules)));
    defaults::install_with_prompt_capability(&mut builder, ModelProviderPlugin::new);
    defaults::mock_model_hosts(&mut builder);
    defaults::tool_host(&mut builder, tools);
    builder.build()
}
fn config(policy: PermissionPolicy) -> SessionConfig {
    SessionConfig {
        tools: TOOLS.iter().map(|(name, _)| name.to_string()).collect(),
        permission_policy: policy,
        ..SessionConfig::default()
    }
}
fn preset(preset: PermissionPreset) -> PermissionPolicy {
    PermissionPolicy {
        preset,
        ..PermissionPolicy::default()
    }
}
/// Runs `tool:<name>` on main; returns whether its result was an error.
async fn call(session: &Session, name: &str) -> bool {
    session
        .submit(format!("tool:{name} {{}}"), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    last_result(session, "main").await.is_error == Some(true)
}
/// Submits `exec` with `command` on main.
async fn submit_exec(session: &Session, command: &str) {
    let input = serde_json::json!({ "command": command });
    session
        .submit(format!("tool:exec {input}"), InputMode::Queue, "main")
        .await
        .unwrap();
}
/// Runs `exec` with `command`; returns its result.
async fn exec(session: &Session, command: &str) -> ToolResult {
    submit_exec(session, command).await;
    session.idle("main").await.unwrap();
    last_result(session, "main").await
}
/// Waits for the approval request of a call with `command` on main.
async fn approval_request(session: &Session, command: &str) -> String {
    loop {
        let found = session
            .local("main", ..)
            .await
            .unwrap()
            .iter()
            .find_map(|event| match &event.kind {
                EventKind::ApprovalRequest(data)
                    if data
                        .call
                        .input
                        .get("command")
                        .and_then(|value| value.as_str())
                        == Some(command) =>
                {
                    Some(data.approval_id.clone())
                }
                _ => None,
            });
        if let Some(id) = found {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
async fn approval_requests(session: &Session) -> usize {
    session
        .local("main", ..)
        .await
        .unwrap()
        .iter()
        .filter(|event| matches!(event.kind, EventKind::ApprovalRequest(_)))
        .count()
}
fn text(result: &ToolResult) -> &str {
    match &result.content {
        ToolContent::Text(text) => text,
        other => panic!("text result expected: {other:?}"),
    }
}
async fn last_result(session: &Session, branch: &str) -> ToolResult {
    session
        .history(branch, None)
        .await
        .unwrap()
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ToolResult(data) => Some(data.results[0].result.clone()),
            _ => None,
        })
        .unwrap()
}
/// Decides a call of `name` without input.
async fn verdict(permission: &Permission, name: &str, risk: Option<&str>) -> &'static str {
    let call = ApprovalCallData {
        name: name.into(),
        input: JsonObject::new(),
    };
    match permission
        .decide(&call, risk, &CancellationToken::new())
        .await
        .unwrap()
    {
        Decision::Allow => "allow",
        Decision::Ask { .. } => "ask",
        Decision::Deny { .. } => "deny",
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn presets_decide_by_risk_and_denied_calls_do_not_run() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let tools = Arc::new(RiskTools::default());
        let agent = agent(tools.clone());
        // read, write, exec, mcp, skill, plain (unclassified)
        let expected = [
            (
                PermissionPreset::ReadOnly,
                ["allow", "deny", "deny", "deny", "allow", "deny"],
            ),
            (
                PermissionPreset::WorkspaceWrite,
                ["allow", "allow", "ask", "ask", "allow", "ask"],
            ),
            (PermissionPreset::FullAccess, ["allow"; 6]),
        ];
        for (policy, verdicts) in expected {
            let session = agent.create_session(config(preset(policy))).await.unwrap();
            let permission = session.service::<Permission>("main").unwrap();
            for ((name, risk), expected) in TOOLS.iter().zip(verdicts) {
                assert_eq!(
                    verdict(&permission, name, *risk).await,
                    expected,
                    "{policy:?} {name}"
                );
            }
            session.close().await.unwrap();
        }
        // A denied call is a tool error and never reaches its handler.
        let session = agent
            .create_session(config(PermissionPolicy::default()))
            .await
            .unwrap();
        assert!(call(&session, "write").await);
        let denied = last_result(&session, "main").await;
        assert_eq!(
            denied.content,
            ToolContent::Text(
                "Tool `write` is not allowed by the read-only preset (writing files)".into()
            )
        );
        assert!(!call(&session, "read").await);
        assert_eq!(tools.calls.load(Ordering::SeqCst), 1);
        session.close().await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .expect("permission flow completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_lists_override_presets_and_disallowed_wins() {
    let agent = agent(Arc::new(RiskTools::default()));
    let cases = [
        (
            PermissionPolicy {
                preset: PermissionPreset::FullAccess,
                allowed_tools: vec![],
                disallowed_tools: vec!["read".into()],
            },
            "deny",
        ),
        (
            PermissionPolicy {
                preset: PermissionPreset::ReadOnly,
                allowed_tools: vec!["exec".into()],
                disallowed_tools: vec![],
            },
            "allow",
        ),
        (
            PermissionPolicy {
                preset: PermissionPreset::FullAccess,
                allowed_tools: vec!["read".into(), "exec".into()],
                disallowed_tools: vec!["read".into(), "exec".into()],
            },
            "deny",
        ),
    ];
    for (policy, expected) in cases {
        let name = policy
            .disallowed_tools
            .first()
            .or(policy.allowed_tools.first())
            .unwrap()
            .clone();
        let risk = TOOLS.iter().find(|(tool, _)| *tool == name).unwrap().1;
        let session = agent.create_session(config(policy)).await.unwrap();
        let permission = session.service::<Permission>("main").unwrap();
        assert_eq!(verdict(&permission, &name, risk).await, expected);
        session.close().await.unwrap();
    }
    agent.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn updates_are_versioned_per_branch_take_effect_when_dispatched_and_survive_restore() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let tools = Arc::new(RiskTools::default());
        let agent = agent(tools.clone());
        let session = agent
            .create_session(config(preset(PermissionPreset::FullAccess)))
            .await
            .unwrap();
        let permission = session.service::<Permission>("main").unwrap();
        let changes = async |session: &Session, branch: &str| {
            session
                .local(branch, ..)
                .await
                .unwrap()
                .iter()
                .filter(|event| matches!(event.kind, EventKind::PermissionChanged(_)))
                .count()
        };

        // A call admitted before the update runs; calls after it use the new policy.
        session
            .submit("tool:hold {}".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        tools.entered.notified().await;
        let version = permission
            .update(0, preset(PermissionPreset::ReadOnly))
            .await
            .unwrap();
        assert!(version > 0);
        assert_eq!(
            session.get::<PermissionState>("main").unwrap().unwrap(),
            PermissionState {
                version,
                policy: preset(PermissionPreset::ReadOnly)
            }
        );
        tools.release.notify_one();
        session.idle("main").await.unwrap();
        assert_eq!(
            last_result(&session, "main").await.content,
            ToolContent::Text("hold ran".into())
        );
        assert!(call(&session, "exec").await);

        // A stale version writes nothing; of two updates on one version one wins.
        assert_eq!(
            permission
                .update(0, preset(PermissionPreset::FullAccess))
                .await,
            Err(PermissionUpdateError::Conflict { current: version })
        );
        assert_eq!(changes(&session, "main").await, 1);
        let (first, second) = tokio::join!(
            permission.update(version, preset(PermissionPreset::WorkspaceWrite)),
            permission.update(version, preset(PermissionPreset::FullAccess)),
        );
        let (won, lost) = match (first, second) {
            (Ok(won), lost) | (lost, Ok(won)) => (won, lost),
            other => panic!("one update must win: {other:?}"),
        };
        assert_eq!(lost, Err(PermissionUpdateError::Conflict { current: won }));
        assert_eq!(changes(&session, "main").await, 2);
        let current = session.get::<PermissionState>("main").unwrap().unwrap();
        assert_eq!(current.version, won);

        // A child inherits the policy at its fork with version 0 and changes alone.
        let tail = session.sequence("main").unwrap();
        let child = session
            .branch(
                "main".into(),
                tail,
                BranchOptions {
                    prompt: "child".into(),
                    source: BranchSource::Host {
                        operation_id: "permission-child".into(),
                    },
                },
            )
            .await
            .unwrap();
        let inherited = session.get::<PermissionState>(&child).unwrap().unwrap();
        assert_eq!(
            inherited,
            PermissionState {
                version: 0,
                policy: current.policy.clone()
            }
        );
        let child_permission = session.service::<Permission>(&child).unwrap();
        child_permission
            .update(0, preset(PermissionPreset::ReadOnly))
            .await
            .unwrap();
        assert_eq!(
            session.get::<PermissionState>("main").unwrap().unwrap(),
            current
        );

        // The policy is rebuilt from history.
        let id = session.id().to_owned();
        session.close().await.unwrap();
        drop(session);
        let restored = agent.restore_session(id).await.unwrap();
        assert_eq!(
            restored.get::<PermissionState>("main").unwrap().unwrap(),
            current
        );
        restored.close().await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .expect("permission flow completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_allowed_approval_is_rechecked_against_the_current_policy() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let tools = Arc::new(RiskTools::default());
        let agent = agent(tools.clone());
        let session = agent
            .create_session(config(preset(PermissionPreset::WorkspaceWrite)))
            .await
            .unwrap();
        session
            .submit("tool:exec {}".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let approval_id = loop {
            let found = session
                .local("main", ..)
                .await
                .unwrap()
                .iter()
                .find_map(|event| match &event.kind {
                    EventKind::ApprovalRequest(data) => Some(data.approval_id.clone()),
                    _ => None,
                });
            if let Some(id) = found {
                break id;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        let permission = session.service::<Permission>("main").unwrap();
        permission
            .update(
                0,
                PermissionPolicy {
                    preset: PermissionPreset::WorkspaceWrite,
                    allowed_tools: vec![],
                    disallowed_tools: vec!["exec".into()],
                },
            )
            .await
            .unwrap();
        let approval = session.service::<Approval>("main").unwrap();
        approval
            .respond(&approval_id, ApprovalDecision::Allow)
            .await
            .unwrap();
        session.idle("main").await.unwrap();
        assert!(last_result(&session, "main").await.is_error == Some(true));
        assert!(
            session
                .local("main", ..)
                .await
                .unwrap()
                .iter()
                .any(|event| matches!(
                    &event.kind,
                    EventKind::ApprovalCancelled(data) if data.approval_id == approval_id
                ))
        );
        assert_eq!(tools.calls.load(Ordering::SeqCst), 0);
        session.close().await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .expect("approval flow completed");
}

/// Commits an `allow` for every pending approval while `stop/interrupt` is
/// dispatched, after ToolExecutor cancelled the batch and before the admit
/// waiting on it runs: the response then finds its request cancelled.
struct RespondOnStop;
impl rutis::TypedPlugin for RespondOnStop {
    type Deps = (
        rutis::Keyed<Dispatcher>,
        rutis::Keyed<dim_agent_capabilities::approval::Approval>,
    );
    fn name(&self) -> &str {
        "RespondOnStop"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (rutis::Keyed(dispatcher), rutis::Keyed(approval)): Self::Deps,
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        use futures_util::FutureExt;
        Box::pin(async move {
            dispatcher.subscribe(ctx, "RespondOnStop", &["stop/interrupt"], move |_| {
                let mut pending = Vec::new();
                approval.replay(|request| pending.push(request.approval_id.clone()));
                for id in pending {
                    // Only the commit matters; the response settles later.
                    let _ = approval
                        .respond(&id, ApprovalDecision::Allow)
                        .now_or_never();
                }
                Box::pin(async { Ok(()) })
            })?;
            Ok(rutis::Effect::Done)
        })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_response_that_finds_its_batch_stopped_ends_as_an_interrupt() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let tools = Arc::new(RiskTools::default());
        let mut builder = AgentBuilder::new(AgentConfig::default());
        builder.timeline_store(Arc::new(MemoryTimelineStore::default()));
        defaults::install(&mut builder);
        defaults::mock_model_hosts(&mut builder);
        defaults::tool_host(&mut builder, tools.clone());
        builder.branch_part(|names| names.part(RespondOnStop));
        let agent = builder.build();
        let session = agent
            .create_session(config(preset(PermissionPreset::WorkspaceWrite)))
            .await
            .unwrap();
        session
            .submit("tool:exec {}".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let approval = session.service::<Approval>("main").unwrap();
        loop {
            let mut pending = false;
            approval.replay(|_| pending = true);
            if pending {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        session.stop("main", "user stop".into()).await.unwrap();
        session.idle("main").await.unwrap();
        let events = session.local("main", ..).await.unwrap();
        let outcome = events
            .iter()
            .rev()
            .find_map(|event| match &event.kind {
                EventKind::TurnEnd(data) => Some(data.outcome.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            outcome,
            TurnOutcome::Cancelled {
                reason: "user stop".into()
            }
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, EventKind::ApprovalRespond(_)))
        );
        assert_eq!(tools.calls.load(Ordering::SeqCst), 0);
        session.close().await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .expect("stopped approval settled");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn allow_rules_answer_approval_only_for_simple_commands_within_the_preset() {
    use CommandMatch::Prefix;
    use CommandRuleDecision::Allow;
    tokio::time::timeout(Duration::from_secs(10), async {
        let tools = Arc::new(RiskTools::default());
        let rules = TestRules::new(vec![rule("cargo test", Prefix, Allow)]);
        let agent = ruled_agent(tools.clone(), rules);
        let session = agent
            .create_session(config(preset(PermissionPreset::WorkspaceWrite)))
            .await
            .unwrap();
        assert_eq!(text(&exec(&session, "cargo test -p x").await), "exec ran");
        assert_eq!(approval_requests(&session).await, 0);
        // Not simple, or not on a word boundary: the preset asks.
        let approval = session.service::<Approval>("main").unwrap();
        for command in ["cargo test; rm x", "cargo test > out", "cargo testx"] {
            submit_exec(&session, command).await;
            let id = approval_request(&session, command).await;
            approval.respond(&id, ApprovalDecision::Deny).await.unwrap();
            session.idle("main").await.unwrap();
        }
        assert_eq!(approval_requests(&session).await, 3);
        session.close().await.unwrap();

        // An allow rule never lifts a preset denial.
        let session = agent
            .create_session(config(preset(PermissionPreset::ReadOnly)))
            .await
            .unwrap();
        assert_eq!(
            text(&exec(&session, "cargo test").await),
            "Tool `exec` is not allowed by the read-only preset (starting processes)"
        );
        assert_eq!(tools.calls.load(Ordering::SeqCst), 1);
        session.close().await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .expect("allow rule flow completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deny_rules_match_any_segment_under_every_preset_and_are_read_at_use_time() {
    use CommandMatch::Prefix;
    use CommandRuleDecision::{Allow, Deny};
    tokio::time::timeout(Duration::from_secs(10), async {
        let tools = Arc::new(RiskTools::default());
        let rules = TestRules::new(vec![]);
        let agent = ruled_agent(tools.clone(), rules.clone());
        let session = agent
            .create_session(config(preset(PermissionPreset::FullAccess)))
            .await
            .unwrap();
        assert_eq!(text(&exec(&session, "git push origin").await), "exec ran");
        // The next call reads the new rules; nothing reloads.
        rules.set(Some(vec![
            rule("git push", Prefix, Deny),
            rule("git push", Prefix, Allow),
        ]));
        for command in ["git push origin", "echo ok && git push", "x=$(git push)"] {
            let result = exec(&session, command).await;
            assert_eq!(result.is_error, Some(true), "{command}");
            assert_eq!(
                text(&result),
                "Command denied by command rule `git push` (prefix)"
            );
        }
        assert_eq!(text(&exec(&session, "git status").await), "exec ran");
        assert_eq!(tools.calls.load(Ordering::SeqCst), 2);
        session.close().await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .expect("deny rule flow completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_deny_rule_written_while_approval_is_pending_cancels_the_allowed_call() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let tools = Arc::new(RiskTools::default());
        let rules = TestRules::new(vec![]);
        let agent = ruled_agent(tools.clone(), rules.clone());
        let session = agent
            .create_session(config(preset(PermissionPreset::WorkspaceWrite)))
            .await
            .unwrap();
        submit_exec(&session, "git push").await;
        let id = approval_request(&session, "git push").await;
        rules.set(Some(vec![rule(
            "git push",
            CommandMatch::Exact,
            CommandRuleDecision::Deny,
        )]));
        let approval = session.service::<Approval>("main").unwrap();
        approval
            .respond(&id, ApprovalDecision::Allow)
            .await
            .unwrap();
        session.idle("main").await.unwrap();
        let reason = "Command denied by command rule `git push` (exact)";
        assert_eq!(text(&last_result(&session, "main").await), reason);
        assert!(
            session
                .local("main", ..)
                .await
                .unwrap()
                .iter()
                .any(|event| matches!(
                    &event.kind,
                    EventKind::ApprovalCancelled(data)
                        if data.approval_id == id && data.reason == reason
                ))
        );
        assert_eq!(tools.calls.load(Ordering::SeqCst), 0);
        session.close().await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .expect("recheck flow completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unreadable_rules_deny_command_calls_only() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let tools = Arc::new(RiskTools::default());
        let rules = TestRules::new(vec![]);
        rules.set(None);
        let agent = ruled_agent(tools.clone(), rules);
        let session = agent
            .create_session(config(preset(PermissionPreset::FullAccess)))
            .await
            .unwrap();
        assert_eq!(
            text(&exec(&session, "ls").await),
            "Command rules could not be read: invalid rules file"
        );
        assert!(!call(&session, "read").await);
        assert_eq!(tools.calls.load(Ordering::SeqCst), 1);
        session.close().await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .expect("unreadable rules flow completed");
}

/// No rules; while `hold` is set a read waits for `release`.
#[derive(Default)]
struct HeldRules {
    hold: std::sync::atomic::AtomicBool,
    entered: Notify,
    release: Notify,
}
impl ReadCommandRules for HeldRules {
    fn rules(&self, _: CancellationToken) -> HostFuture<'_, Vec<CommandRule>> {
        Box::pin(async move {
            if self.hold.load(Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(Vec::new())
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_policy_tightened_during_the_rules_read_decides_the_call() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let tools = Arc::new(RiskTools::default());
        let rules = Arc::new(HeldRules::default());
        let agent = ruled_agent(tools.clone(), rules.clone());

        // First decision: FullAccess when the read starts, ReadOnly when it ends.
        let session = agent
            .create_session(config(preset(PermissionPreset::FullAccess)))
            .await
            .unwrap();
        rules.hold.store(true, Ordering::SeqCst);
        submit_exec(&session, "ls").await;
        rules.entered.notified().await;
        let permission = session.service::<Permission>("main").unwrap();
        permission
            .update(0, preset(PermissionPreset::ReadOnly))
            .await
            .unwrap();
        rules.release.notify_one();
        session.idle("main").await.unwrap();
        assert_eq!(
            text(&last_result(&session, "main").await),
            "Tool `exec` is not allowed by the read-only preset (starting processes)"
        );
        session.close().await.unwrap();

        // Recheck after an allow response: the tool is disallowed meanwhile.
        rules.hold.store(false, Ordering::SeqCst);
        let session = agent
            .create_session(config(preset(PermissionPreset::WorkspaceWrite)))
            .await
            .unwrap();
        submit_exec(&session, "ls").await;
        let id = approval_request(&session, "ls").await;
        rules.hold.store(true, Ordering::SeqCst);
        let approval = session.service::<Approval>("main").unwrap();
        approval
            .respond(&id, ApprovalDecision::Allow)
            .await
            .unwrap();
        rules.entered.notified().await;
        let permission = session.service::<Permission>("main").unwrap();
        permission
            .update(
                0,
                PermissionPolicy {
                    preset: PermissionPreset::WorkspaceWrite,
                    allowed_tools: vec![],
                    disallowed_tools: vec!["exec".into()],
                },
            )
            .await
            .unwrap();
        rules.release.notify_one();
        session.idle("main").await.unwrap();
        assert_eq!(
            text(&last_result(&session, "main").await),
            "Tool `exec` is disallowed by the permission policy"
        );
        assert_eq!(tools.calls.load(Ordering::SeqCst), 0);
        session.close().await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .expect("policy race flow completed");
}
