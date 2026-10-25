//! Integration tests for the persistent shell execution capability.
//!
//! Covers the acceptance scenarios from docs/design/appserver/persistent-shell.md §10.

#![cfg(unix)]
#[path = "local_support/process.rs"]
mod process_support;
use dim_agent_capabilities::{
    adapters::DefaultPrepareModelPlugin,
    file_storage::FileStoragePlugin,
    history::HistoryRenderer,
    host::*,
    host_platform::{HostPlatform, HostPlatformConfig, HostPlatformPlugin, resolve_shell},
    model_provider::ModelProviderPlugin,
    permission::{CommandRulesHost, NoCommandRules},
    skill_settings::SkillSettingsHost,
    skills::{MemorySkillSettings, SkillsCatalogPlugin},
};
use dim_agent_runtime::*;
use dim_agent_types::*;
use process_support::{alive, gone, pid_in};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypeKey, TypedPlugin};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

// ------------------------------------------------------------------ model

/// Replies with queued tool batches, then with text.
#[derive(Default)]
struct Script {
    replies: Mutex<VecDeque<Vec<(String, Value)>>>,
    count: AtomicUsize,
}
impl Script {
    fn push(&self, calls: &[(&str, Value)]) {
        self.replies.lock().unwrap().push_back(
            calls
                .iter()
                .map(|(name, input)| (name.to_string(), input.clone()))
                .collect(),
        );
    }
}
impl ModelDriver for Script {
    fn invoke(
        &self,
        _request: Arc<ModelRequest>,
        _: ModelConnection,
        _: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        Box::pin(async move {
            let id = format!("m{}", self.count.fetch_add(1, Ordering::Relaxed));
            let mut chunks = vec![Ok(ModelChunk::MessageStart {
                message_id: id.clone(),
            })];
            let reason = match self.replies.lock().unwrap().pop_front() {
                Some(calls) => {
                    for (index, (name, input)) in calls.into_iter().enumerate() {
                        let Value::Object(input) = input else {
                            unreachable!()
                        };
                        chunks.push(Ok(ModelChunk::ToolStart {
                            id: format!("{id}-{index}"),
                            name: name.clone(),
                        }));
                        chunks.push(Ok(ModelChunk::ToolUse(ToolCall {
                            id: format!("{id}-{index}"),
                            name,
                            input,
                        })));
                    }
                    ModelStopReason::ToolUse
                }
                None => {
                    chunks.push(Ok(ModelChunk::Text("done".into())));
                    chunks.push(Ok(ModelChunk::TextEnd));
                    ModelStopReason::EndTurn
                }
            };
            chunks.push(Ok(ModelChunk::MessageStop {
                message_id: id,
                reason,
            }));
            Ok(Box::pin(futures_util::stream::iter(chunks)) as ModelStream)
        })
    }
}

// ------------------------------------------------------------------ harness

/// Provides the process HostPlatform the design lets tests construct.
struct PlatformPlugin(Arc<HostPlatform>);
impl TypedPlugin for PlatformPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "TestHostPlatform"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as(TypeKey::of::<HostPlatform>(), self.0.clone())?;
            Ok(Effect::Done)
        })
    }
}

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "dim-minimal-exec-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).unwrap();
    std::fs::canonicalize(path).unwrap()
}

async fn platform(extra: &[(&str, &str)]) -> HostPlatform {
    let mut env: Vec<_> = std::env::vars_os().collect();
    env.retain(|(key, _)| key != "SHELL");
    env.push(("SHELL".into(), "/bin/bash".into()));
    let mut platform = HostPlatform::current(resolve_shell(Ok(None), &env).await).unwrap();
    platform.env.extend(
        extra
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value))),
    );
    platform
}

struct Harness {
    root: Ctx,
    agent: Agent,
    model: Arc<Script>,
    cwd: PathBuf,
}

impl Harness {
    async fn new(mut platform: HostPlatform) -> Self {
        platform.home = scratch("home");
        let cwd = scratch("cwd");
        platform.env.retain(|(name, _)| name != "HOME");
        platform.env.push(("HOME".into(), cwd.as_os_str().into()));
        let root = Ctx::root_with(tokio::runtime::Handle::current());
        let view = root.plugin(rutis::Typed::new(PlatformPlugin(Arc::new(platform))));
        Self::with_platform(root, view, cwd).await
    }
    async fn with_platform(root: Ctx, view: rutis::FiberView, cwd: PathBuf) -> Self {
        wait_active(
            &root,
            &[view],
            Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        root.provide_as(
            TypeKey::of::<SkillSettingsHost>(),
            Arc::new(SkillSettingsHost(Arc::new(MemorySkillSettings::default()))),
        )
        .unwrap();
        (&root.plugin(
            dim_agent_runtime::names::process_plugin(
                &dim_agent_capabilities::catalog(),
                SkillsCatalogPlugin::new(),
            )
            .unwrap(),
        ))
            .await
            .unwrap();
        let model = Arc::new(Script::default());
        let mut builder = AgentBuilder::new(AgentConfig::default());
        builder
            .timeline_store(Arc::new(MemoryTimelineStore::default()))
            .session_value(Arc::new(HistoryRenderer::default()))
            .process_value(Arc::new(ModelHost(model.clone())))
            .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
            .session_value(Arc::new(ContextModelHost(None)))
            .session_value(Arc::new(CommandRulesHost(Arc::new(NoCommandRules))))
            .session_value(Arc::new(CompactionStrategyHost(None)))
            .session_part(|names| names.part(DefaultPrepareModelPlugin::new(names)?));
        dim_agent_capabilities::defaults::install_minimal_execution(
            &mut builder,
            ModelProviderPlugin::new,
        );
        let agent = builder.build_in(&root);
        Self {
            root,
            agent,
            model,
            cwd,
        }
    }
    fn config(&self) -> SessionConfig {
        SessionConfig {
            cwd: self.cwd.to_string_lossy().into_owned(),
            tools: Some(vec!["execute".into()]),
            permission_policy: PermissionPolicy {
                preset: PermissionPreset::FullAccess,
                ..Default::default()
            },
            ..SessionConfig::default()
        }
    }
    async fn session(&self) -> Session {
        self.agent.create_session(self.config()).await.unwrap()
    }
    async fn run(&self, session: &Session, calls: &[(&str, Value)]) {
        self.model.push(calls);
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
    }
    async fn shutdown(self) {
        self.agent.close().await.unwrap();
        self.root.shutdown().await.unwrap();
    }
}

async fn local_events(session: &Session) -> Vec<Arc<AgentEvent>> {
    session.local("main", ..).await.unwrap()
}

async fn until(
    session: &Session,
    what: &str,
    done: impl Fn(&[Arc<AgentEvent>]) -> bool,
) -> Vec<Arc<AgentEvent>> {
    for _ in 0..1500 {
        let events = local_events(session).await;
        if done(&events) {
            return events;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!(
        "timed out waiting for {what}: {:#?}",
        local_events(session).await
    );
}

fn results(events: &[Arc<AgentEvent>]) -> Vec<ToolResult> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ToolResult(data) => Some(data.results.iter().map(|r| r.result.clone())),
            _ => None,
        })
        .flatten()
        .collect()
}

fn text(result: &ToolResult) -> &str {
    match &result.content {
        ToolContent::Text(text) => text,
        ToolContent::Blocks(_) => panic!("execute returns text"),
    }
}

fn count(events: &[Arc<AgentEvent>], key: &str) -> usize {
    events
        .iter()
        .filter(|event| event.kind.key() == key)
        .count()
}

fn ended(events: &[Arc<AgentEvent>]) -> Vec<(Arc<AgentEvent>, ExecEndedData)> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ExecEnded(data) => Some((event.clone(), data.clone())),
            _ => None,
        })
        .collect()
}

fn notifications(events: &[Arc<AgentEvent>]) -> Vec<(String, String)> {
    let tag = |event: &AgentEvent, prefix: &str| {
        event
            .tags
            .iter()
            .flatten()
            .find_map(|tag| tag.strip_prefix(prefix).map(str::to_owned))
    };
    events
        .iter()
        .filter(|event| matches!(event.kind, EventKind::InputMessage(_)))
        .filter_map(|event| {
            Some((
                tag(event, "exec-wake-task:")?,
                tag(event, "exec-wake-result:")?,
            ))
        })
        .collect()
}

fn task_of(result: &ToolResult) -> String {
    let text = text(result);
    let start = text
        .find("Task ID: ")
        .expect("running content names the task")
        + "Task ID: ".len();
    text[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect()
}

fn tool_results(events: &[Arc<AgentEvent>]) -> usize {
    count(events, "step/tool-result")
}

// ------------------------------------------------------------------ tests

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn persistent_isolation_preserves_policy_and_job_control() {
    let Some(home) = std::env::var_os("DIM_TEST_ISOLATED_SHELL_HOME") else {
        for mode in [
            "default",
            "configured",
            "read-only",
            "workspace-only",
            "disabled",
        ] {
            let home = scratch("isolated-node");
            let policy = match mode {
                "default" => None,
                "configured" => Some(json!({
                    "version": 1,
                    "writablePaths": [home.join("workspace"), home.join(".dim/exec")]
                })),
                "read-only" => {
                    let read_only: Vec<_> = [
                        "/usr",
                        "/bin",
                        "/sbin",
                        "/lib",
                        "/lib64",
                        "/etc",
                        "/System",
                        "/Library/Apple",
                        "/private/etc",
                    ]
                    .into_iter()
                    .map(PathBuf::from)
                    .filter(|path| path.is_dir())
                    .chain([home.clone()])
                    .collect();
                    Some(json!({"version":1,"readOnlyPaths":read_only,"writablePaths":[]}))
                }
                "workspace-only" => {
                    Some(json!({"version":1,"writablePaths":[home.join("workspace")]}))
                }
                "disabled" => Some(json!({"version":1,"enabled":false})),
                _ => unreachable!(),
            };
            if let Some(policy) = policy {
                std::fs::write(home.join("host-isolation.json"), policy.to_string()).unwrap();
            }
            let output = tokio::time::timeout(
                Duration::from_secs(45),
                tokio::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "persistent_isolation_preserves_policy_and_job_control",
                        "--nocapture",
                    ])
                    .env("DIM_TEST_ISOLATED_SHELL_HOME", &home)
                    .env("DIM_TEST_SHELL_POLICY", mode)
                    .env("HOME", &home)
                    .current_dir(&home)
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .unwrap()
            .unwrap();
            std::fs::remove_dir_all(home).unwrap();
            assert!(
                output.status.success(),
                "policy={mode}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    };
    let home = PathBuf::from(home);
    let policy = home.join("host-isolation.json");
    let mode = std::env::var("DIM_TEST_SHELL_POLICY").unwrap();
    let initially_enabled = mode != "disabled";
    let cwd = home.join("workspace");
    let output_root = home.join(".dim/exec");
    let private_file = home.join("private-note");
    std::fs::write(&private_file, "owner-home-value").unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&output_root).unwrap();
    std::fs::write(
        home.join("exec-shell.json"),
        r#"{"version":1,"executablePath":"/bin/bash"}"#,
    )
    .unwrap();
    let root = Ctx::root_with(tokio::runtime::Handle::current());
    let storage = root.plugin(rutis::Typed::new(FileStoragePlugin::new(vec![
        home.clone(),
    ])));
    (&storage).await.unwrap();
    let view = root.plugin(
        HostPlatformPlugin::new(HostPlatformConfig {
            exec_shell_file: Some((0, "exec-shell.json".into())),
            isolation_file: Some((0, "host-isolation.json".into())),
        })
        .typed(&dim_agent_capabilities::catalog())
        .unwrap(),
    );
    let harness = Harness::with_platform(root, view, cwd).await;
    let session = harness.session().await;
    let stop = session
        .service::<dyn dim_agent_capabilities::exec_stop::ExecStop>("main")
        .unwrap();
    if matches!(mode.as_str(), "read-only" | "workspace-only") {
        let marker = harness.cwd.join("write-attempt");
        for background in [false, true] {
            harness
                .run(
                    &session,
                    &[(
                        "execute",
                        json!({
                            "command":format!("printf executed > {}", marker.display()),
                            "runInBackground":background
                        }),
                    )],
                )
                .await;
            session.idle("main").await.unwrap();
            let result = results(&local_events(&session).await).pop().unwrap();
            if background {
                let task = task_of(&result);
                let events = until(&session, "denied background redirection", |events| {
                    ended(events).iter().any(|(_, end)| end.task_id == task)
                })
                .await;
                let (_, end) = ended(&events)
                    .into_iter()
                    .find(|(_, end)| end.task_id == task)
                    .unwrap();
                assert_eq!(end.status, ExecStatus::Failed);
                assert!(!marker.exists());
            } else if mode == "read-only" {
                assert_eq!(result.is_error, Some(true), "{}", text(&result));
                assert!(!marker.exists());
            } else {
                assert_ne!(result.is_error, Some(true), "{}", text(&result));
                assert_eq!(std::fs::read_to_string(&marker).unwrap(), "executed");
                std::fs::remove_file(&marker).unwrap();
            }
        }
        session.close().await.unwrap();
        harness.shutdown().await;
        return;
    }
    let mut shell_temporary = None;
    if mode == "default" {
        // The shell owns one TMPDIR for its lifetime.
        let (content, path) = (
            harness.cwd.join("shell-temporary"),
            harness.cwd.join("tmpdir"),
        );
        harness
            .run(
                &session,
                &[(
                    "execute",
                    json!({"command": format!(
                        "f=$(mktemp) && printf shell-temporary > \"$f\" && cp \"$f\" {} \
                         && printf %s \"$TMPDIR\" > {}",
                        content.display(),
                        path.display()
                    )}),
                )],
            )
            .await;
        session.idle("main").await.unwrap();
        let result = results(&local_events(&session).await).pop().unwrap();
        assert_ne!(result.is_error, Some(true), "{}", text(&result));
        assert_eq!(std::fs::read_to_string(content).unwrap(), "shell-temporary");
        let directory = PathBuf::from(std::fs::read_to_string(path).unwrap());
        assert!(directory.is_dir());
        shell_temporary = Some(directory);
        let sibling = output_root.join(session.id()).join("sibling");
        std::fs::create_dir_all(&sibling).unwrap();
        let sibling_file = sibling.join("output.log");
        std::fs::write(&sibling_file, "sibling-output").unwrap();
        for command in [
            format!("cat {}", sibling_file.display()),
            format!("printf overwritten > {}", sibling_file.display()),
        ] {
            harness
                .run(&session, &[("execute", json!({"command":command}))])
                .await;
            session.idle("main").await.unwrap();
            let result = results(&local_events(&session).await).pop().unwrap();
            assert_eq!(result.is_error, Some(true), "{}", text(&result));
            assert!(!text(&result).contains("sibling-output"));
        }
        assert_eq!(
            std::fs::read_to_string(sibling_file).unwrap(),
            "sibling-output"
        );
    }
    harness
        .run(
            &session,
            &[(
                "execute",
                json!({"command":"test -t 0 && test -t 1 && stty rows 41 cols 132 && stty size"}),
            )],
        )
        .await;
    session.idle("main").await.unwrap();
    let terminal_result = results(&local_events(&session).await).pop().unwrap();
    let foreground_pid_file = harness.cwd.join("foreground.pid");
    harness
        .run(
            &session,
            &[(
                "execute",
                json!({
                    "command":format!("sh -c 'echo $$ > {}; sleep 100'", foreground_pid_file.display()),
                    "timeoutMs":20
                }),
            )],
        )
        .await;
    session.idle("main").await.unwrap();
    let foreground = task_of(results(&local_events(&session).await).last().unwrap());
    let foreground_pid = pid_in(&foreground_pid_file).await;
    let foreground_stopped = stop.clone().stop(foreground.clone()).await;
    let foreground_gone = gone(&foreground_pid).await;
    harness
        .run(
            &session,
            &[(
                "execute",
                json!({"command":"test -t 0 && test -t 1 && stty rows 41 cols 132 && stty size && printf terminal-alive"}),
            )],
        )
        .await;
    session.idle("main").await.unwrap();
    let terminal_after_stop = results(&local_events(&session).await).pop().unwrap();
    let cleanup = harness.cwd.join("first-released");
    let mut jobs = Vec::new();
    let mut snapshot_result = None;
    for name in ["first", "second"] {
        let pid_file = harness.cwd.join(format!("{name}.pid"));
        let command = if name == "first" {
            format!(
                "printf first-output; sh -c 'trap \"sleep 0.1; printf released > {}; exit 0\" TERM; echo $$ > {}; while :; do sleep 1; done'",
                cleanup.display(),
                pid_file.display()
            )
        } else {
            format!("sh -c 'sleep 100 & echo $! > {}; wait'", pid_file.display())
        };
        harness
            .run(
                &session,
                &[(
                    "execute",
                    json!({
                        "command":command,
                        "runInBackground":true
                    }),
                )],
            )
            .await;
        session.idle("main").await.unwrap();
        let events = local_events(&session).await;
        let task = task_of(results(&events).last().unwrap());
        let pid = pid_in(&pid_file).await;
        assert!(alive(&pid));
        jobs.push((task, pid));
        if name == "first" {
            std::fs::write(&policy, r#"{"version":1,"enabled":false}"#).unwrap();
            harness
                .run(
                    &session,
                    &[(
                        "execute",
                        json!({"command": format!("cat {}", private_file.display())}),
                    )],
                )
                .await;
            session.idle("main").await.unwrap();
            snapshot_result = results(&local_events(&session).await).pop();
        }
    }
    let stopped = stop.stop(jobs[0].0.clone()).await;
    let cleanup_completed = cleanup.exists();
    let selected_gone = gone(&jobs[0].1).await;
    let sibling_alive = alive(&jobs[1].1);
    assert!(
        std::fs::read_to_string(
            output_root
                .join(session.id())
                .join("main")
                .join(format!("{}.log", jobs[0].0))
        )
        .unwrap()
        .contains("first-output")
    );
    let closed = session.close().await;
    let sibling_gone = gone(&jobs[1].1).await;
    let events = local_events(&session).await;
    harness.shutdown().await;
    for result in [terminal_result, terminal_after_stop] {
        assert_ne!(result.is_error, Some(true), "{}", text(&result));
        assert!(text(&result).contains("41 132"), "{}", text(&result));
    }
    assert!(
        foreground_stopped
            .as_ref()
            .is_ok_and(|end| end.status == ExecStatus::Cancelled)
            && foreground_gone,
        "foreground_stop={foreground_stopped:?}, foreground_gone={foreground_gone}"
    );
    let snapshot_result = snapshot_result.unwrap();
    assert_eq!(snapshot_result.is_error == Some(true), initially_enabled);
    assert_eq!(
        text(&snapshot_result).contains("owner-home-value"),
        !initially_enabled,
        "{}",
        text(&snapshot_result)
    );
    assert!(
        stopped
            .as_ref()
            .is_ok_and(|end| end.status == ExecStatus::Cancelled)
            && cleanup_completed
            && selected_gone
            && sibling_alive
            && sibling_gone
            && closed.is_ok(),
        "stop={stopped:?}, cleanup_completed={cleanup_completed}, selected_gone={selected_gone}, sibling_alive={sibling_alive}, sibling_gone={sibling_gone}, close={closed:?}"
    );
    for task in std::iter::once(foreground).chain(jobs.into_iter().map(|(task, _)| task)) {
        assert_eq!(
            ended(&events)
                .iter()
                .filter(|(_, end)| end.task_id == task)
                .count(),
            1
        );
    }
    if let Some(directory) = shell_temporary {
        tokio::time::timeout(Duration::from_secs(5), async {
            while directory.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the shell's TMPDIR is removed with its PTY");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn close_settles_every_admitted_shell_task() {
    let harness = Harness::new(platform(&[]).await).await;
    let session = harness.session().await;
    harness
        .run(
            &session,
            &[(
                "execute",
                json!({"command": "sleep 60", "runInBackground": true}),
            )],
        )
        .await;
    until(&session, "background admission", |events| {
        tool_results(events) == 1
    })
    .await;
    harness
        .run(
            &session,
            &[("execute", json!({"command": "sleep 60", "timeoutMs": 20}))],
        )
        .await;
    until(&session, "occupying admission", |events| {
        count(events, "exec/started") == 2 && tool_results(events) == 2
    })
    .await;
    session.close().await.unwrap();
    let events = local_events(&session).await;
    for start in events.iter().filter_map(|event| match &event.kind {
        EventKind::ExecStarted(data) => Some(data),
        _ => None,
    }) {
        assert_eq!(
            ended(&events)
                .iter()
                .filter(|(_, end)| end.task_id == start.task_id)
                .count(),
            1,
            "admitted task {} must settle exactly once",
            start.task_id
        );
    }
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn new_persistent_shells_and_environment_refresh_use_current_preferences() {
    let home = scratch("shell-config");
    let config = home.join("exec-shell.json");
    let set_shell = |shell: &str| {
        std::fs::write(
            &config,
            json!({"version":1,"executablePath":shell}).to_string(),
        )
        .unwrap()
    };
    set_shell("/bin/bash");
    let root = Ctx::root_with(tokio::runtime::Handle::current());
    let storage = root.plugin(rutis::Typed::new(FileStoragePlugin::new(vec![
        home.clone(),
    ])));
    (&storage).await.unwrap();
    let view = root.plugin(
        HostPlatformPlugin::new(HostPlatformConfig {
            exec_shell_file: Some((0, "exec-shell.json".into())),
            ..HostPlatformConfig::default()
        })
        .typed(&dim_agent_capabilities::catalog())
        .unwrap(),
    );
    let harness = Harness::with_platform(root, view, scratch("cwd")).await;
    let session = harness.session().await;
    let alternate = if Path::new("/bin/zsh").is_file() {
        "/bin/zsh"
    } else {
        "/usr/bin/bash"
    };
    // The first shell is created after the service and Session were published.
    set_shell(alternate);
    session
        .configure(
            dim_agent_capabilities::prompt_sources::refresh_update(),
            "main",
        )
        .await
        .unwrap();
    harness
        .run(
            &session,
            &[("execute", json!({"command":"printf 'selected=%s' \"$0\""}))],
        )
        .await;
    session.idle("main").await.unwrap();
    let events = local_events(&session).await;
    assert!(text(results(&events).last().unwrap()).contains(&format!("selected={alternate}")));
    let environment = events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::InputMessage(input) => input.blocks.iter().find_map(|block| match block {
                InputBlock::Raw { text, .. }
                    if text.contains("<dim-state source=\"dim/environment/context\">") =>
                {
                    Some(text)
                }
                _ => None,
            }),
            _ => None,
        })
        .unwrap();
    assert!(
        environment.contains(&format!("<shell>{alternate}</shell>")),
        "{environment}"
    );
    // Editing the preference does not replace a running shell.
    set_shell("/bin/bash");
    harness
        .run(
            &session,
            &[("execute", json!({"command":"printf 'retained=%s' \"$0\""}))],
        )
        .await;
    session.idle("main").await.unwrap();
    assert!(
        text(results(&local_events(&session).await).last().unwrap())
            .contains(&format!("retained={alternate}"))
    );
    harness
        .run(&session, &[("execute", json!({"command":"exit 7"}))])
        .await;
    session.idle("main").await.unwrap();
    // A replacement shell reads the current preference, without a Session reload.
    harness
        .run(
            &session,
            &[("execute", json!({"command":"printf 'reset=%s' \"$0\""}))],
        )
        .await;
    session.idle("main").await.unwrap();
    let events = local_events(&session).await;
    assert!(text(results(&events).last().unwrap()).contains("reset=/bin/bash"));
    harness.shutdown().await;
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn foreground_echo_returns_output() {
    let harness = Harness::new(platform(&[]).await).await;
    let session = harness.session().await;
    harness
        .run(&session, &[("execute", json!({"command": "echo hello"}))])
        .await;
    let events = until(&session, "tool result", |events| tool_results(events) == 1).await;
    let result = &results(&events)[0];
    assert!(text(result).contains("hello"), "{}", text(result));
    assert_eq!(result.is_error, None);
    // The declaration states the default isolation.
    let declared = events
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => data
                .request
                .tools
                .as_ref()?
                .iter()
                .find_map(|tool| (tool.name == "execute").then(|| tool.description.clone())),
            _ => None,
        })
        .unwrap();
    assert!(
        declared.contains("Isolation: unless the user disabled it")
            && declared.contains("host-isolation.json"),
        "{declared}"
    );
    session.close().await.unwrap();
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failing_command_reports_exit_code() {
    let harness = Harness::new(platform(&[]).await).await;
    let session = harness.session().await;
    harness
        .run(
            &session,
            &[("execute", json!({"command": "sh -c 'exit 3'"}))],
        )
        .await;
    let events = until(&session, "tool result", |events| tool_results(events) == 1).await;
    let result = &results(&events)[0];
    assert_eq!(result.is_error, Some(true));
    assert!(text(result).contains("exit code 3"), "{}", text(result));
    session.close().await.unwrap();
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn state_persists_across_calls() {
    let harness = Harness::new(platform(&[]).await).await;
    std::fs::create_dir(harness.cwd.join("child")).unwrap();
    let session = harness.session().await;
    harness
        .run(
            &session,
            &[
                ("execute", json!({"command": "export FOO=bar && cd child"})),
                ("execute", json!({"command": "echo $FOO && pwd"})),
            ],
        )
        .await;
    let events = until(&session, "tool results", |events| {
        results(events).len() == 2
    })
    .await;
    let result = &results(&events)[1];
    assert!(text(result).contains("bar"), "{}", text(result));
    assert!(
        text(result).contains(&*harness.cwd.join("child").to_string_lossy()),
        "{}",
        text(result)
    );
    session.close().await.unwrap();
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_task_returns_task_id_and_completes() {
    let harness = Harness::new(platform(&[]).await).await;
    let session = harness.session().await;
    harness
        .run(
            &session,
            &[(
                "execute",
                json!({"command": "true", "runInBackground": true}),
            )],
        )
        .await;
    let events = until(&session, "exec started", |events| {
        count(events, "exec/started") == 1 && tool_results(events) == 1
    })
    .await;
    let result = &results(&events)[0];
    let task_id = task_of(result);
    assert!(!task_id.is_empty());
    assert!(
        text(result).contains("Background command started."),
        "{}",
        text(result)
    );

    // Wait for completion.
    let events = until(&session, "exec ended", |events| ended(events).len() == 1).await;
    let (end_event, end) = &ended(&events)[0];
    assert_eq!(end.status, ExecStatus::Completed);
    assert_eq!(end.task_id, task_id);
    let start = events
        .iter()
        .find(|event| {
            matches!(&event.kind,
        EventKind::ExecStarted(data) if data.task_id == task_id)
        })
        .unwrap();
    assert!(
        start.sequence < end_event.sequence,
        "completion preceded admission"
    );

    // Wake notification.
    let events = until(&session, "wake", |events| !notifications(events).is_empty()).await;
    assert_eq!(
        notifications(&events),
        vec![(task_id.clone(), end_event.id.clone())]
    );
    session.close().await.unwrap();
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn timeout_occupies_shell_and_wakes() {
    let harness = Harness::new(platform(&[]).await).await;
    let session = harness.session().await;
    harness
        .run(
            &session,
            &[(
                "execute",
                json!({"command": "sleep 0.5 && echo done", "timeoutMs": 100}),
            )],
        )
        .await;
    let events = until(&session, "exec started", |events| {
        count(events, "exec/started") == 1 && tool_results(events) == 1
    })
    .await;
    let result = &results(&events)[0];
    let task_id = task_of(result);
    assert!(!task_id.is_empty());
    assert!(
        text(result).contains("occupies the shell"),
        "{}",
        text(result)
    );

    // Wait for completion.
    let events = until(&session, "exec ended", |events| ended(events).len() == 1).await;
    let (end_event, end) = &ended(&events)[0];
    assert_eq!(end.status, ExecStatus::Completed);
    assert_eq!(end.task_id, task_id);

    // Wake notification.
    let events = until(&session, "wake", |events| !notifications(events).is_empty()).await;
    assert_eq!(
        notifications(&events),
        vec![(task_id.clone(), end_event.id.clone())]
    );
    session.close().await.unwrap();
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_background_task() {
    let harness = Harness::new(platform(&[]).await).await;
    let session = harness.session().await;
    let pid_file = harness.cwd.join("bg.pid");
    harness
        .run(
            &session,
            &[(
                "execute",
                json!({
                    "command": format!("sh -c 'sleep 100 & echo $! > {}; wait'", pid_file.display()),
                    "runInBackground": true
                }),
            )],
        )
        .await;
    let events = until(&session, "start", |events| count(events, "turn/end") == 1).await;
    let task_id = task_of(&results(&events)[0]);
    let pid = pid_in(&pid_file).await;

    // Stop via the branch service.
    let stop = session
        .service::<dyn dim_agent_capabilities::exec_stop::ExecStop>("main")
        .unwrap();
    let ended = stop.stop(task_id.clone()).await.unwrap();
    assert_eq!(ended.status, ExecStatus::Cancelled);
    assert!(gone(&pid).await);
    session.close().await.unwrap();
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shell_exit_settles_tasks() {
    let harness = Harness::new(platform(&[]).await).await;
    let session = harness.session().await;
    harness
        .run(&session, &[("execute", json!({"command": "exit 7"}))])
        .await;
    let events = until(&session, "tool result", |events| tool_results(events) == 1).await;
    let result = &results(&events)[0];
    assert!(
        text(result).contains("[shell exited: code 7]"),
        "{}",
        text(result)
    );

    // Next call resets the shell.
    harness
        .run(&session, &[("execute", json!({"command": "echo after"}))])
        .await;
    let events = until(&session, "reset", |events| tool_results(events) == 2).await;
    let result = &results(&events)[1];
    assert!(text(result).contains("after"), "{}", text(result));
    assert!(
        text(result).contains("[shell was reset]"),
        "{}",
        text(result)
    );
    // EOF can precede process exit: close the PTY, then exit later. The
    // session must wait for the status rather than killing the live process.
    harness
        .run(
            &session,
            &[(
                "execute",
                json!({"command": "exec /bin/sh -c 'exec </dev/null >/dev/null 2>&1; sleep 0.2; exit 7'"}),
            )],
        )
        .await;
    let events = until(&session, "exit after EOF", |events| {
        tool_results(events) == 3
    })
    .await;
    let result = &results(&events)[2];
    assert!(
        text(result).contains("[shell exited: code 7]"),
        "{}",
        text(result)
    );
    session.close().await.unwrap();
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn completion_markers_stay_out_of_captures() {
    let harness = Harness::new(platform(&[]).await).await;
    let session = harness.session().await;
    let background = json!({"command": "sleep 0.2", "runInBackground": true});

    // A background job finishes while a foreground command runs.
    harness
        .run(&session, &[("execute", background.clone())])
        .await;
    until(&session, "start", |events| tool_results(events) == 1).await;
    harness
        .run(
            &session,
            &[("execute", json!({"command": "sleep 0.6 && echo fg"}))],
        )
        .await;
    let events = until(&session, "foreground", |events| tool_results(events) == 2).await;
    let result = &results(&events)[1];
    assert!(text(result).contains("fg"), "{}", text(result));
    assert!(!text(result).contains("__DIM_"), "{}", text(result));

    // A background job finishes while an occupying task runs.
    harness.run(&session, &[("execute", background)]).await;
    until(&session, "start", |events| tool_results(events) == 3).await;
    harness
        .run(
            &session,
            &[(
                "execute",
                json!({"command": "sleep 0.6 && echo occ", "timeoutMs": 100}),
            )],
        )
        .await;
    let events = until(&session, "occupying end", |events| ended(events).len() == 3).await;
    let occupying = task_of(&results(&events)[3]);
    let (_, end) = ended(&events)
        .into_iter()
        .find(|(_, data)| data.task_id == occupying)
        .unwrap();
    assert_eq!(end.status, ExecStatus::Completed);
    assert!(text(&end.result).contains("occ"), "{}", text(&end.result));
    let output_path = end.result.structured_content.as_ref().unwrap()["outputPath"]
        .as_str()
        .unwrap()
        .to_owned();
    let output = std::fs::read_to_string(output_path).unwrap();
    assert!(output.contains("occ"), "{output}");
    assert!(!output.contains("__DIM_"), "{output}");
    session.close().await.unwrap();
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn occupying_task_settles_after_large_output() {
    let harness = Harness::new(platform(&[]).await).await;
    let session = harness.session().await;
    harness
        .run(
            &session,
            &[(
                "execute",
                json!({"command": "sleep 0.3 && seq 1 30000 && echo tail", "timeoutMs": 100}),
            )],
        )
        .await;
    let events = until(&session, "exec ended", |events| ended(events).len() == 1).await;
    let (_, end) = &ended(&events)[0];
    assert_eq!(end.status, ExecStatus::Completed);
    let output_path = end.result.structured_content.as_ref().unwrap()["outputPath"]
        .as_str()
        .unwrap()
        .to_owned();
    let output = std::fs::read_to_string(output_path).unwrap();
    assert!(
        output.ends_with("30000\ntail\n"),
        "{}",
        &output[output.len() - 100..]
    );
    assert!(!output.contains("__DIM_"));
    session.close().await.unwrap();
    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shell_exit_kills_background_jobs() {
    let harness = Harness::new(platform(&[]).await).await;
    let session = harness.session().await;
    let pid_file = harness.cwd.join("bg.pid");
    harness
        .run(
            &session,
            &[(
                "execute",
                json!({
                    "command": format!("sh -c 'sleep 100 & echo $! > {}; wait'", pid_file.display()),
                    "runInBackground": true
                }),
            )],
        )
        .await;
    let events = until(&session, "start", |events| tool_results(events) == 1).await;
    let task_id = task_of(&results(&events)[0]);
    let pid = pid_in(&pid_file).await;

    harness
        .run(&session, &[("execute", json!({"command": "exit 7"}))])
        .await;
    let events = until(&session, "job settled", |events| ended(events).len() == 1).await;
    let (_, end) = &ended(&events)[0];
    assert_eq!(end.task_id, task_id);
    assert_eq!(end.status, ExecStatus::Failed);
    assert!(
        text(&end.result).contains("shell was reset"),
        "{}",
        text(&end.result)
    );
    assert!(gone(&pid).await);
    session.close().await.unwrap();
    harness.shutdown().await;
}
