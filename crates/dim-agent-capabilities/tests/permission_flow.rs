//! Branch permission through the public Session interfaces.
use dim_agent_capabilities::{
    defaults,
    host::*,
    permission::{Decision, Permission, PermissionState, PermissionUpdateError},
    tool_registry::definition,
};
use dim_agent_runtime::*;
use dim_agent_types::*;
use std::{
    sync::{
        Arc,
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

fn agent(tools: Arc<RiskTools>) -> Agent {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.timeline_store(Arc::new(MemoryTimelineStore::default()));
    defaults::install(&mut builder);
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
fn verdict(decision: Decision) -> &'static str {
    match decision {
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
                    verdict(permission.decide(name, *risk)),
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
        assert_eq!(verdict(permission.decide(&name, risk)), expected);
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
    use dim_agent_capabilities::approval::Approval;
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
struct RespondOnStop {
    scope: BranchScope,
}
impl rutis::TypedPlugin for RespondOnStop {
    type Deps = (
        InBranch<Dispatcher>,
        InBranch<dim_agent_capabilities::approval::Approval>,
    );
    fn name(&self) -> &str {
        "RespondOnStop"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (InBranch(dispatcher), InBranch(approval)): Self::Deps,
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
impl Capability for RespondOnStop {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_response_that_finds_its_batch_stopped_ends_as_an_interrupt() {
    use dim_agent_capabilities::approval::Approval;
    tokio::time::timeout(Duration::from_secs(10), async {
        let tools = Arc::new(RiskTools::default());
        let mut builder = AgentBuilder::new(AgentConfig::default());
        builder.timeline_store(Arc::new(MemoryTimelineStore::default()));
        defaults::install(&mut builder);
        defaults::mock_model_hosts(&mut builder);
        defaults::tool_host(&mut builder, tools.clone());
        builder.timeline_plugin(|scope| RespondOnStop { scope: *scope });
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
