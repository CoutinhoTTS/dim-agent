//! Integration tests for the persistent shell execution capability.
//!
//! Covers the acceptance scenarios from docs/design/appserver/persistent-shell.md §10.

#![cfg(unix)]
use dim_agent_capabilities::{
    adapters::DefaultPrepareModelPlugin,
    file_storage::FileStoragePlugin,
    history::HistoryRenderer,
    host::*,
    host_platform::{HostPlatform, HostPlatformConfig, HostPlatformPlugin, resolve_shell},
    model_provider::ModelProviderPlugin,
    skill_settings::SkillSettingsHost,
    skills::{MemorySkillSettings, SkillsCatalogPlugin},
};
use dim_agent_runtime::*;
use dim_agent_types::*;
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
impl Capability for PlatformPlugin {
    fn keys(&self) -> KeysOf<Self> {}
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
        let root = Ctx::root_with(tokio::runtime::Handle::current());
        let view = root.plugin(capability_plugin(PlatformPlugin(Arc::new(platform))));
        Self::with_platform(root, view).await
    }
    async fn with_platform(root: Ctx, view: rutis::FiberView) -> Self {
        let cwd = scratch("cwd");
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
        (&root.plugin(capability_plugin(SkillsCatalogPlugin::new())))
            .await
            .unwrap();
        let model = Arc::new(Script::default());
        let mut builder = AgentBuilder::new(AgentConfig::default());
        builder
            .timeline_store(Arc::new(MemoryTimelineStore::default()))
            .session_value(Arc::new(HistoryRenderer::default()))
            .session_value(Arc::new(ModelHost(model.clone())))
            .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
            .session_value(Arc::new(ContextModelHost(None)))
            .session_value(Arc::new(CompactionStrategyHost(None)))
            .session_plugin(DefaultPrepareModelPlugin::new);
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
            tools: vec!["execute".into()],
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

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

async fn gone(pid: &str) -> bool {
    for _ in 0..100 {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// Waits for the pid a command wrote into `file`.
async fn pid_in(file: &Path) -> String {
    for _ in 0..500 {
        if let Ok(text) = std::fs::read_to_string(file)
            && text.ends_with('\n')
        {
            return text.trim().to_owned();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no pid in {}", file.display());
}

fn tool_results(events: &[Arc<AgentEvent>]) -> usize {
    count(events, "step/tool-result")
}

// ------------------------------------------------------------------ tests

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
    let storage = root.plugin(capability_plugin(FileStoragePlugin::new(vec![
        home.clone(),
    ])));
    (&storage).await.unwrap();
    let view = root.plugin(capability_plugin(HostPlatformPlugin::new(
        HostPlatformConfig {
            exec_shell_file: Some((0, "exec-shell.json".into())),
        },
    )));
    let harness = Harness::with_platform(root, view).await;
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
    let session = harness.session().await;
    harness
        .run(
            &session,
            &[
                ("execute", json!({"command": "export FOO=bar && cd /tmp"})),
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
    assert!(text(result).contains("/tmp"), "{}", text(result));
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
