//! Branch permission through the public Session interfaces.
use dim_agent_capabilities::{
    approval::Approval,
    defaults, file_access,
    host::*,
    host_platform::{HostPlatform, ShellStatus},
    model_provider::ModelProviderPlugin,
    permission::{
        CommandRulesHost, Decision, NoCommandRules, Permission, PermissionState,
        PermissionUpdateError, ReadCommandRules,
    },
    prompt::InitialPromptConfiguration,
    session_grants::{ReadSessionGrants, SessionGrantsHost},
    tool_registry::definition,
};
use dim_agent_runtime::testing::MinimalHost;
use dim_agent_runtime::*;
use dim_agent_types::{
    grant::{GrantKind, GrantMode, SessionGrant},
    *,
};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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

/// The Session's grants as the test sets them; `None` is an unreadable
/// source.
struct TestGrants(Mutex<Option<Vec<SessionGrant>>>);
impl TestGrants {
    fn new(grants: Vec<SessionGrant>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Some(grants))))
    }
    fn set(&self, grants: Option<Vec<SessionGrant>>) {
        *self.0.lock().unwrap() = grants;
    }
}
impl ReadSessionGrants for TestGrants {
    fn grants(&self, _: CancellationToken) -> HostFuture<'_, Vec<SessionGrant>> {
        let grants = self.0.lock().unwrap().clone();
        Box::pin(async move { grants.ok_or_else(|| HostError::failed("invalid grants store")) })
    }
}

/// The Session's owned artifact directories as the test sets them; `fails`
/// is an unreadable ownership query.
#[derive(Default)]
struct TestFiles {
    owned: Mutex<Vec<PathBuf>>,
    fails: AtomicBool,
}
impl TestFiles {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn own(&self, path: impl Into<PathBuf>) {
        self.owned.lock().unwrap().push(path.into());
    }
    fn fail(&self, fails: bool) {
        self.fails.store(fails, Ordering::SeqCst);
    }
}
impl SessionFilesService for TestFiles {
    fn put(
        &self,
        _: String,
        _: String,
        _: Vec<u8>,
        _: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<StoredSessionFile, SessionFileError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async {
            Err(SessionFileError {
                code: SessionFileErrorCode::NodeUnavailable,
                message: "the test stores no files".into(),
            })
        })
    }
    fn owns(
        &self,
        _: String,
        path: PathBuf,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<bool, SessionFileError>> + Send + '_>>
    {
        let owned = self.owned.lock().unwrap().clone();
        let fails = self.fails.load(Ordering::SeqCst);
        Box::pin(async move {
            if fails {
                return Err(SessionFileError {
                    code: SessionFileErrorCode::Storage,
                    message: "invalid ownership store".into(),
                });
            }
            Ok(owned.iter().any(|root| path.starts_with(root)))
        })
    }
}
fn grant(path: impl Into<PathBuf>, kind: GrantKind, mode: GrantMode) -> SessionGrant {
    SessionGrant {
        path: path.into(),
        kind,
        mode,
    }
}
fn rule(command: &str, matching: CommandMatch, decision: CommandRuleDecision) -> CommandRule {
    CommandRule {
        command: command.into(),
        matching,
        decision,
    }
}

async fn agent(tools: Arc<RiskTools>) -> MinimalHost {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.timeline_store(Arc::new(MemoryTimelineStore::default()));
    defaults::install(&mut builder);
    defaults::mock_model_hosts(&mut builder);
    defaults::tool_host(&mut builder, tools);
    MinimalHost::start(&builder).await.unwrap()
}
/// The default profile with `rules` bound in place of its empty host.
async fn ruled_agent(tools: Arc<RiskTools>, rules: Arc<dyn ReadCommandRules>) -> MinimalHost {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder
        .timeline_store(Arc::new(MemoryTimelineStore::default()))
        .session_value(Arc::new(PromptHost(Arc::new(EmptyPromptHost))))
        .session_value(Arc::new(InitialPromptConfiguration::fixed(None)))
        .session_value(Arc::new(CommandRulesHost(rules)));
    defaults::install_with_prompt_capability(&mut builder, ModelProviderPlugin::new);
    defaults::mock_model_hosts(&mut builder);
    defaults::tool_host(&mut builder, tools);
    MinimalHost::start(&builder).await.unwrap()
}
/// The same profile with grants, the fixed facts and their `sessionFiles`
/// source bound.
async fn granted_agent(
    tools: Arc<RiskTools>,
    grants: Arc<dyn ReadSessionGrants>,
    files: Arc<TestFiles>,
) -> MinimalHost {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder
        .timeline_store(Arc::new(MemoryTimelineStore::default()))
        .session_value(Arc::new(PromptHost(Arc::new(EmptyPromptHost))))
        .session_value(Arc::new(InitialPromptConfiguration::fixed(None)))
        .session_value(Arc::new(CommandRulesHost(Arc::new(NoCommandRules))))
        .session_value(Arc::new(SessionGrantsHost(grants)))
        .process_value(Arc::new(SessionFilesHost(files)))
        .process_value(Arc::new(
            HostPlatform::current(ShellStatus::Unavailable {
                code: "PERMISSION_TEST",
                reason: "The permission flow starts no process".into(),
            })
            .unwrap(),
        ));
    defaults::install_with_prompt_capability(&mut builder, ModelProviderPlugin::new);
    file_access::install(&mut builder);
    defaults::mock_model_hosts(&mut builder);
    defaults::tool_host(&mut builder, tools);
    MinimalHost::start(&builder).await.unwrap()
}
fn config(policy: PermissionPolicy) -> SessionConfig {
    SessionConfig {
        tools: Some(TOOLS.iter().map(|(name, _)| name.to_string()).collect()),
        permission_policy: policy,
        ..SessionConfig::default()
    }
}
fn config_in(cwd: &Path, policy: PermissionPolicy) -> SessionConfig {
    SessionConfig {
        cwd: cwd.to_string_lossy().into_owned(),
        ..config(policy)
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
/// Decides a `write` call for `path`.
async fn decide_write(permission: &Permission, path: &Path) -> Decision {
    let mut input = JsonObject::new();
    input.insert("path".into(), path.to_string_lossy().into_owned().into());
    permission
        .decide(
            &ApprovalCallData {
                name: "write".into(),
                input,
            },
            Some("write_filesystem"),
            &CancellationToken::new(),
        )
        .await
        .unwrap()
}
/// Decides a `write` call for `path`; returns its verdict.
async fn write_verdict(permission: &Permission, path: &Path) -> &'static str {
    match decide_write(permission, path).await {
        Decision::Allow => "allow",
        Decision::Ask { .. } => "ask",
        Decision::Deny { .. } => "deny",
    }
}
fn temporary_directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("dim-permission-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    std::fs::canonicalize(path).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_composition_without_facts_builds_and_asks() {
    let work = temporary_directory("no-grants");
    let agent = agent(Arc::new(RiskTools::default())).await;
    let session = agent
        .create_session(config_in(&work, preset(PermissionPreset::WorkspaceWrite)))
        .await
        .unwrap();
    let permission = session.service::<Permission>("main").unwrap();
    assert_eq!(
        write_verdict(&permission, &work.join("inside.txt")).await,
        "allow"
    );
    let target = work.parent().unwrap().join("outside.txt");
    assert!(matches!(
        decide_write(&permission, &target).await,
        Decision::Ask { .. }
    ));
    session.close().await.unwrap();
    agent.close().await.unwrap();
    let _ = std::fs::remove_dir_all(work);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_write_facts_decide_each_path_by_preset() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let root = temporary_directory("facts");
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let outside = root.join("outside.txt");
        let owned = root.join("owned").join("artifact.txt");
        std::fs::create_dir_all(owned.parent().unwrap()).unwrap();
        std::fs::write(&owned, b"owned").unwrap();
        let system =
            PathBuf::from("/usr/bin").join(format!("dim-permission-{}", uuid::Uuid::new_v4()));
        let grants = TestGrants::new(Vec::new());
        let files = TestFiles::new();
        files.own(owned.clone());
        let agent = granted_agent(Arc::new(RiskTools::default()), grants, files.clone()).await;

        let inside = work.join("file.txt");
        for (policy, expected) in [
            (PermissionPreset::ReadOnly, ["deny", "deny", "deny", "deny"]),
            (
                PermissionPreset::WorkspaceWrite,
                ["allow", "ask", "deny", "deny"],
            ),
            (
                PermissionPreset::FullAccess,
                ["allow", "allow", "allow", "allow"],
            ),
        ] {
            let session = agent
                .create_session(config_in(&work, preset(policy)))
                .await
                .unwrap();
            let permission = session.service::<Permission>("main").unwrap();
            for (path, expected) in [&inside, &outside, &owned, &system]
                .into_iter()
                .zip(expected)
            {
                assert_eq!(
                    write_verdict(&permission, path).await,
                    expected,
                    "{policy:?} {}",
                    path.display()
                );
            }
            session.close().await.unwrap();
        }

        // An explicit tool allow does not bypass a path fact.
        let session = agent
            .create_session(config_in(
                &work,
                PermissionPolicy {
                    preset: PermissionPreset::ReadOnly,
                    allowed_tools: vec!["write".into()],
                    ..Default::default()
                },
            ))
            .await
            .unwrap();
        let permission = session.service::<Permission>("main").unwrap();
        assert_eq!(write_verdict(&permission, &inside).await, "allow");
        assert_eq!(write_verdict(&permission, &owned).await, "deny");
        assert_eq!(write_verdict(&permission, &system).await, "deny");
        session.close().await.unwrap();

        // An unreadable ownership or grants source denies instead of asking.
        files.fail(true);
        let session = agent
            .create_session(config_in(&work, preset(PermissionPreset::WorkspaceWrite)))
            .await
            .unwrap();
        let permission = session.service::<Permission>("main").unwrap();
        for path in [&inside, &outside] {
            assert_eq!(write_verdict(&permission, path).await, "deny");
        }
        session.close().await.unwrap();
        agent.close().await.unwrap();
        let _ = std::fs::remove_dir_all(root);
    })
    .await
    .expect("file facts flow completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_grant_matches_only_the_canonical_target_and_mode() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let root = temporary_directory("grants");
        let work = root.join("work");
        let granted = root.join("granted");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&granted).unwrap();
        let grants = TestGrants::new(Vec::new());
        let files = TestFiles::new();
        let tools = Arc::new(RiskTools::default());
        let agent = granted_agent(tools.clone(), grants.clone(), files).await;
        let session = agent
            .create_session(config_in(&work, preset(PermissionPreset::WorkspaceWrite)))
            .await
            .unwrap();
        let permission = session.service::<Permission>("main").unwrap();
        let target = granted.join("new.txt");

        assert!(matches!(
            decide_write(&permission, &target).await,
            Decision::Ask { .. }
        ));
        grants.set(Some(vec![grant(
            &target,
            GrantKind::File,
            GrantMode::Write,
        )]));
        assert_eq!(decide_write(&permission, &target).await, Decision::Allow);

        // A read grant cannot authorize a write.
        grants.set(Some(vec![grant(&target, GrantKind::File, GrantMode::Read)]));
        assert!(matches!(
            decide_write(&permission, &target).await,
            Decision::Ask { .. }
        ));

        // A directory grant covers children but not a `..` escape.
        grants.set(Some(vec![grant(
            &granted,
            GrantKind::Dir,
            GrantMode::Write,
        )]));
        assert_eq!(
            decide_write(&permission, &granted.join("child.txt")).await,
            Decision::Allow
        );
        assert!(matches!(
            decide_write(&permission, &granted.join("..").join("escaped.txt")).await,
            Decision::Ask { .. }
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            // A symlink is matched by its canonical target: a file grant on
            // the target admits the link, and a directory grant does not
            // cover a link that resolves outside it.
            let real = granted.join("real.txt");
            let link = granted.join("link.txt");
            std::fs::write(&real, b"real").unwrap();
            symlink(&real, &link).unwrap();
            grants.set(Some(vec![grant(&real, GrantKind::File, GrantMode::Write)]));
            assert_eq!(decide_write(&permission, &link).await, Decision::Allow);

            let outside = root.join("outside");
            let escape = granted.join("escape");
            std::fs::create_dir_all(&outside).unwrap();
            symlink(&outside, &escape).unwrap();
            grants.set(Some(vec![grant(
                &granted,
                GrantKind::Dir,
                GrantMode::Write,
            )]));
            assert!(matches!(
                decide_write(&permission, &escape.join("child.txt")).await,
                Decision::Ask { .. }
            ));
        }

        // The end-to-end call re-checks through Permission and runs without
        // an approval prompt.
        grants.set(Some(vec![grant(
            &granted,
            GrantKind::Dir,
            GrantMode::Write,
        )]));
        let input = serde_json::json!({"path": target});
        session
            .submit(format!("tool:write {input}"), InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
        assert_ne!(last_result(&session, "main").await.is_error, Some(true));
        assert_eq!(approval_requests(&session).await, 0);
        assert_eq!(tools.calls.load(Ordering::SeqCst), 1);

        // A grant source that cannot be read denies instead of asking.
        grants.set(None);
        assert!(matches!(
            decide_write(&permission, &target).await,
            Decision::Deny { .. }
        ));

        session.close().await.unwrap();
        agent.close().await.unwrap();
        let _ = std::fs::remove_dir_all(root);
    })
    .await
    .expect("grant flow completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn presets_decide_by_risk_and_denied_calls_do_not_run() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let tools = Arc::new(RiskTools::default());
        let agent = agent(tools.clone()).await;
        let expected = [
            (
                PermissionPreset::ReadOnly,
                ["allow", "deny", "deny", "deny", "allow", "deny"],
            ),
            (
                PermissionPreset::WorkspaceWrite,
                ["allow", "ask", "ask", "ask", "allow", "ask"],
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
    let agent = agent(Arc::new(RiskTools::default())).await;
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
        let agent = agent(tools.clone()).await;
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
        let agent = agent(tools.clone()).await;
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
    dispatcher: rutis::host::Key<Dispatcher>,
    approval: rutis::host::Key<Approval>,
}
impl RespondOnStop {
    fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            dispatcher: names.key()?,
            approval: names.key()?,
        })
    }
}
impl rutis::host::DynPlugin for RespondOnStop {
    fn name(&self) -> &str {
        "RespondOnStop"
    }
    fn injects(&self) -> Vec<rutis::host::TypeKey> {
        vec![self.dispatcher.erase(), self.approval.erase()]
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
    ) -> rutis::host::BoxFuture<'a, rutis::Result<rutis::host::Effect>> {
        use futures_util::FutureExt;
        use rutis::host::CtxHost;
        Box::pin(async move {
            let dispatcher = ctx.require_at(&self.dispatcher)?;
            let approval = ctx.require_at(&self.approval)?;
            dispatcher.subscribe(ctx, "RespondOnStop", &["stop/interrupt"], move |_| {
                let mut pending = Vec::new();
                approval.replay(|request| pending.push(request.approval_id.clone()));
                for id in pending {
                    let _ = approval
                        .respond(&id, ApprovalDecision::Allow)
                        .now_or_never();
                }
                Box::pin(async { Ok(()) })
            })?;
            Ok(rutis::host::Effect::Done)
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
        builder.require_feature(
            Feature::new("RespondOnStop").branch("RespondOnStop", |names| {
                Ok(Box::new(RespondOnStop::new(names)?))
            }),
        );
        let agent = MinimalHost::start(&builder).await.unwrap();
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
        let agent = ruled_agent(tools.clone(), rules).await;
        let session = agent
            .create_session(config(preset(PermissionPreset::WorkspaceWrite)))
            .await
            .unwrap();
        assert_eq!(text(&exec(&session, "cargo test -p x").await), "exec ran");
        assert_eq!(approval_requests(&session).await, 0);
        let approval = session.service::<Approval>("main").unwrap();
        for command in ["cargo test; rm x", "cargo test > out", "cargo testx"] {
            submit_exec(&session, command).await;
            let id = approval_request(&session, command).await;
            approval.respond(&id, ApprovalDecision::Deny).await.unwrap();
            session.idle("main").await.unwrap();
        }
        assert_eq!(approval_requests(&session).await, 3);
        session.close().await.unwrap();

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
        let agent = ruled_agent(tools.clone(), rules.clone()).await;
        let session = agent
            .create_session(config(preset(PermissionPreset::FullAccess)))
            .await
            .unwrap();
        assert_eq!(text(&exec(&session, "git push origin").await), "exec ran");
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
        let agent = ruled_agent(tools.clone(), rules.clone()).await;
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
        let agent = ruled_agent(tools.clone(), rules).await;
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
        let agent = ruled_agent(tools.clone(), rules.clone()).await;

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
