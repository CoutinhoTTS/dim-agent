//! The local execution product profile end to end: real file tools, exec,
//! Permission and Approval, PromptSources and Skills; only the model is
//! scripted.

mod local_support;

use dim_agent_capabilities::{
    approval::Approval,
    defaults,
    history::HistoryRenderer,
    host::*,
    host_platform::{
        ExecShellRuntime, HostPlatform, OutputEncoding, ShellEdition, ShellSource, ShellStatus,
    },
    model_provider::ModelProviderPlugin,
    permission::{CommandRulesHost, NoCommandRules},
    skill_settings::SkillSettingsHost,
    skills::{MemorySkillSettings, SkillsCatalogPlugin},
    workspace_tools::WorkspaceToolsConfig,
};
use dim_agent_runtime::*;
use dim_agent_types::*;
use local_support::*;
use rutis::TypeKey;
use std::{path::Path, sync::Arc, time::Duration};

struct Product {
    _root: rutis::Ctx,
    agent: Agent,
    model: Arc<ScriptedModel>,
}

async fn product(dirs: &Dirs) -> Product {
    let root = rutis::Ctx::root_with(tokio::runtime::Handle::current());
    let mut platform = HostPlatform::current(ShellStatus::Available(ExecShellRuntime {
        executable_path: "/bin/sh".into(),
        edition: ShellEdition::Sh,
        source: ShellSource::Fallback,
        output_encoding: OutputEncoding::Utf8,
    }))
    .unwrap();
    platform.home = dirs.home.clone();
    root.provide_as(TypeKey::of::<HostPlatform>(), Arc::new(platform))
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
    let model = Arc::new(ScriptedModel::default());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder
        .session_value(Arc::new(HistoryRenderer::default()))
        .process_value(Arc::new(ModelHost(model.clone())))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_value(Arc::new(CommandRulesHost(Arc::new(NoCommandRules))))
        .session_value(Arc::new(CompactionStrategyHost(None)))
        .session_part(|names| {
            names.part(dim_agent_capabilities::adapters::DefaultPrepareModelPlugin::new(names)?)
        });
    defaults::install_local_execution(
        &mut builder,
        ModelProviderPlugin::new,
        WorkspaceToolsConfig {
            output_root: dirs.dir("outputs"),
        },
    );
    Product {
        agent: builder.build_in(&root),
        _root: root,
        model,
    }
}

async fn session(product: &Product, cwd: &Path) -> Session {
    product
        .agent
        .create_session(SessionConfig {
            cwd: cwd.to_string_lossy().into_owned(),
            tools: Some(
                [
                    "read", "write", "edit", "glob", "grep", "exec", "wait", "skill",
                ]
                .map(String::from)
                .to_vec(),
            ),
            permission_policy: PermissionPolicy {
                preset: PermissionPreset::WorkspaceWrite,
                ..Default::default()
            },
            ..SessionConfig::default()
        })
        .await
        .unwrap()
}

/// Waits for the first pending approval of the main branch.
async fn approval_request(session: &Session) -> ApprovalRequestData {
    for _ in 0..400 {
        let pending = session
            .history("main", None)
            .await
            .unwrap()
            .iter()
            .find_map(|event| match &event.kind {
                EventKind::ApprovalRequest(data) => Some(data.clone()),
                _ => None,
            });
        if let Some(data) = pending {
            return data;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no approval request");
}

async fn tool_results(session: &Session) -> Vec<ToolResult> {
    session
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ToolResult(data) => Some(data.results.clone()),
            _ => None,
        })
        .flatten()
        .map(|call| call.result)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_reads_writes_and_runs_an_approved_command_with_real_tools() {
    let dirs = Dirs::new();
    let cwd = dirs.dir("work");
    dirs.write(&cwd.join("AGENTS.md"), "Project rule MARKER-P");
    dirs.write(&cwd.join("input.txt"), "hello\n");
    dirs.write(
        &dirs.home.join(".agents/skills/review/SKILL.md"),
        &skill_file("review", "Review code", "Review body"),
    );
    let product = product(&dirs).await;
    let session = session(&product, &cwd).await;

    product
        .model
        .push(Step::Tool("read", serde_json::json!({"path": "input.txt"})));
    product.model.push(Step::Tool(
        "write",
        serde_json::json!({"path": "notes.txt", "content": "written\n"}),
    ));
    product.model.push(Step::Tool(
        "exec",
        serde_json::json!({"command": "echo ran > ran.txt"}),
    ));
    product.model.push(Step::Text("done"));
    session
        .submit("go".into(), InputMode::Queue, "main")
        .await
        .unwrap();

    // workspace-write: read and write run directly, exec asks.
    let request = approval_request(&session).await;
    assert_eq!(request.call.name, "exec");
    assert!(!cwd.join("ran.txt").exists(), "exec must wait for approval");
    session
        .service::<Approval>("main")
        .unwrap()
        .respond(&request.approval_id, ApprovalDecision::Allow)
        .await
        .unwrap();
    session.idle("main").await.unwrap();

    assert_eq!(
        std::fs::read_to_string(cwd.join("notes.txt")).unwrap(),
        "written\n"
    );
    assert_eq!(
        std::fs::read_to_string(cwd.join("ran.txt")).unwrap(),
        "ran\n"
    );
    let results = tool_results(&session).await;
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|result| result.is_error != Some(true)));

    // The request carried the product context and the fixed tool set.
    let requests = requests(&session).await;
    let first = &requests[0];
    let system = system_text(first.system.as_ref().unwrap());
    let messages = request_text(first);
    assert!(
        !system.contains("MARKER-P") && !system.contains("<cwd>"),
        "the system holds only static content"
    );
    assert!(
        messages
            .contains("<dim-rule source=\"dim/instructions/project\" scope=\"branch\" op=\"set\">")
            && messages.contains("MARKER-P"),
        "AGENTS.md arrives as a rule"
    );
    assert!(messages.contains("<dim-state source=\"dim/environment/context\">"));
    assert!(!system.contains("Review code"), "skills are not in system");
    assert!(
        request_text(first).contains("Review code"),
        "skills arrive as input state"
    );
    let mut declared: Vec<_> = first
        .tools
        .as_ref()
        .unwrap()
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    declared.sort_unstable();
    assert_eq!(
        declared,
        [
            "edit", "exec", "glob", "grep", "read", "skill", "wait", "write"
        ]
    );
    // Every request declares exec with the same static isolation line.
    let exec: Vec<_> = requests
        .iter()
        .map(|request| {
            let tools = request.tools.as_ref().unwrap();
            let exec = tools.iter().find(|tool| tool.name == "exec").unwrap();
            exec.description.clone()
        })
        .collect();
    assert!(exec.len() > 1 && exec.iter().all(|description| *description == exec[0]));
    assert_eq!(
        exec[0].contains("Isolation: unless the user disabled it")
            && exec[0].contains("host-isolation.json"),
        cfg!(unix),
        "{}",
        exec[0]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_denied_command_has_no_side_effect() {
    let dirs = Dirs::new();
    let cwd = dirs.dir("work");
    let product = product(&dirs).await;
    let session = session(&product, &cwd).await;

    product.model.push(Step::Tool(
        "exec",
        serde_json::json!({"command": "echo ran > ran.txt"}),
    ));
    product.model.push(Step::Text("done"));
    session
        .submit("go".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    let request = approval_request(&session).await;
    session
        .service::<Approval>("main")
        .unwrap()
        .respond(&request.approval_id, ApprovalDecision::Deny)
        .await
        .unwrap();
    session.idle("main").await.unwrap();

    assert!(!cwd.join("ran.txt").exists());
    let results = tool_results(&session).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].is_error, Some(true));
}
