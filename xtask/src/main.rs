mod allocation;
mod app_server;
mod catalog;
mod modules;
mod plugins;
mod scenario_eval;
mod summary_eval;
mod types;
#[global_allocator]
static ALLOCATOR: allocation::MeasuredAllocator = allocation::MeasuredAllocator;
use dim_agent_runtime::{testing::MinimalHost, *};
use rutis::host::CtxHost;
use rutis::host::diagnostics::RuntimeDiagnostics;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("graph") => graph().await?,
        Some("measure") => measure().await?,
        Some("summary-eval") => summary_eval::run().await?,
        Some("scenario-eval") => scenario_eval::run().await?,
        Some("check") => {
            boundaries(false)?;
            catalog::check(&root()).await?;
            types::check(&root())?;
        }
        Some("types") => types::write(&root())?,
        _ => {
            return Err(
                "usage: cargo run -p xtask -- graph|measure [--model-config PATH]|summary-eval [OPTIONS]|scenario-eval [OPTIONS]|check|types"
                    .into(),
            );
        }
    }
    Ok(())
}
async fn agent() -> dim_agent_runtime::Result<MinimalHost> {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    MinimalHost::start(&builder).await
}
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}
/// Instance identities are allocated per process; the graph names their scope instead.
fn diagnostics(mut d: RuntimeDiagnostics, scopes: &[(rutis::host::InstanceId, &str)]) -> Value {
    let describe = |key: &rutis::host::TypeKey| {
        let text = key.describe();
        match key.instance_id() {
            Some(id) => {
                let scope = scopes
                    .iter()
                    .find(|(instance, _)| *instance == id)
                    .map_or("?", |(_, name)| name);
                text.replace(&format!("@{id}"), &format!("@{scope}"))
            }
            None => text,
        }
    };
    for p in &mut d.plugins {
        p.injects
            .sort_by_key(|i| (i.key.describe(), i.scope.clone()));
        p.resolved_dependencies
            .sort_by_key(|i| (i.key.describe(), i.scope.clone(), i.provider));
        p.accesses
            .sort_by_key(|i| (i.key.describe(), i.scope.clone(), i.provider));
    }
    d.bindings.sort_by_key(|b| (b.key.describe(), b.provider));
    json!({"shuttingDown":d.shutting_down,"plugins":d.plugins.into_iter().map(|p|json!({
        "id":p.id.0,"parent":p.parent.map(|id|id.0),"name":p.name,"state":format!("{:?}",p.state),"generation":p.generation,
        "injects":p.injects.into_iter().map(|i|json!({"service":describe(&i.key),"scope":i.scope,"status":format!("{:?}",i.status)})).collect::<Vec<_>>(),
        "resolved":p.resolved_dependencies.into_iter().map(|r|json!({"service":describe(&r.key),"scope":r.scope,"provider":r.provider.0,"generation":r.generation})).collect::<Vec<_>>(),
        "reads":p.accesses.into_iter().map(|r|json!({"service":describe(&r.key),"provider":r.provider.map(|id|id.0),"declared":r.declared,"external":r.external})).collect::<Vec<_>>()
    })).collect::<Vec<_>>(),"bindings":d.bindings.into_iter().map(|b|json!({"service":describe(&b.key),"provider":b.provider.0,"generation":b.generation,"removing":b.removing})).collect::<Vec<_>>()})
}
/// The Agent the graphs and measurements run on, in a minimal host, its
/// Session configuration, and whether it runs native models.
async fn configured_agent()
-> std::result::Result<(MinimalHost, SessionConfig, bool), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(2);
    let model_config = match args.next().as_deref() {
        None => None,
        Some("--model-config") => Some(args.next().ok_or("--model-config requires a path")?),
        _ => return Err("graph/measure accept only --model-config PATH".into()),
    };
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let (agent, config, native) = if let Some(path) = &model_config {
        use dim_agent_aimux::{
            AimuxModels, AimuxPlugin, ConfiguredModelsPlugin, config::ModelConfigFile,
        };
        use dim_agent_capabilities::{adapters::MockTools, history::HistoryRenderer, host::*};
        use std::sync::Arc;
        let models = Arc::new(AimuxModels::from_config(ModelConfigFile::load(
            std::path::Path::new(path),
        )?)?);
        let model = models.default_model().clone();
        let mut builder = AgentBuilder::new(AgentConfig {
            model: model.clone(),
            ..Default::default()
        });
        dim_agent_capabilities::defaults::install_with_process_model(&mut builder);
        dim_agent_capabilities::defaults::install_compaction(
            &mut builder,
            models.clone(),
            dim_agent_capabilities::summary_strategy::ProductSummaryStrategyConfig::default(),
        );
        builder
            .session_value(Arc::new(HistoryRenderer::default()))
            .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
            .session_value(Arc::new(ToolHost(Arc::new(MockTools))));
        dim_agent_aimux::install(&mut builder);
        builder.plugin(|_| Ok(Box::new(AimuxPlugin)));
        builder.plugin(move |_| Ok(Box::new(ConfiguredModelsPlugin(models.clone()))));
        (
            MinimalHost::start(&builder).await?,
            SessionConfig {
                model: dim_agent_types::ModelSelection {
                    provider_id: model.provider_id,
                    model_id: model.model_id,
                    capability_snapshot: None,
                    config: None,
                    metadata: None,
                },
                ..Default::default()
            },
            true,
        )
    } else {
        (agent().await?, SessionConfig::default(), false)
    };
    Ok((agent, config, native))
}
/// The host's process plugins in `root`'s snapshot: the root's own children
/// and the loader's rows that provide a service, without the part registry.
fn process_rows(root: &rutis::Ctx) -> RuntimeDiagnostics {
    let mut snapshot = root.diagnostics();
    let id = root.root_view().expect("process root").id;
    let loader = snapshot
        .plugins
        .iter()
        .find(|plugin| plugin.parent == Some(id) && plugin.name == "rutis-loader")
        .map(|plugin| plugin.id);
    let providers: std::collections::BTreeSet<_> = snapshot
        .bindings
        .iter()
        .map(|binding| binding.provider)
        .collect();
    snapshot.plugins.retain(|plugin| {
        plugin.parent == Some(id)
            || (plugin.parent.is_some()
                && plugin.parent == loader
                && providers.contains(&plugin.id)
                && plugin.name != "PartRegistry")
    });
    let members: Vec<_> = snapshot.plugins.iter().map(|plugin| plugin.id).collect();
    snapshot
        .bindings
        .retain(|binding| members.contains(&binding.provider));
    snapshot
}
/// One tree: branch capabilities resolve Session services across the subgraphs.
fn mermaid(graphs: &BTreeMap<&str, Value>) -> String {
    let mut mermaid =
        String::from("%% Generated from live rutis diagnostics; do not edit.\nflowchart LR\n");
    let mut nodes = std::collections::BTreeSet::new();
    for (scope, snapshot) in graphs {
        mermaid += &format!("  subgraph {scope}[\"{scope}\"]\n");
        for p in snapshot["plugins"].as_array().unwrap() {
            nodes.insert(p["id"].as_u64().unwrap());
            mermaid += &format!(
                "    p{}[\"{}\"]\n",
                p["id"],
                p["name"].as_str().unwrap().replace('"', "'")
            );
        }
        mermaid += "  end\n";
    }
    for snapshot in graphs.values() {
        for p in snapshot["plugins"].as_array().unwrap() {
            for edge in p["resolved"].as_array().unwrap() {
                if nodes.contains(&edge["provider"].as_u64().unwrap()) {
                    mermaid += &format!(
                        "  p{} -->|\"{}\"| p{}\n",
                        edge["provider"],
                        edge["service"].as_str().unwrap().replace('"', "'"),
                        p["id"]
                    );
                }
            }
        }
    }
    mermaid
}
async fn graph() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let (agent, config, native) = configured_agent().await?;
    let session = agent.create_session(config).await?;
    let session_diagnostics = session.diagnostics()?;
    let main_diagnostics = session.branch_diagnostics("main")?;
    let instance = |d: &RuntimeDiagnostics, name: &str| {
        let scope = d
            .plugins
            .iter()
            .find(|p| p.name == name)
            .expect("scope plugin");
        d.plugins
            .iter()
            .find(|p| Some(p.id) == scope.parent)
            .map(|p| p.instance)
            .expect("scope group")
    };
    let scopes = [
        (instance(&session_diagnostics, "SessionScope"), "session"),
        (instance(&main_diagnostics, "BranchScope"), "main"),
    ];
    let mut graphs = BTreeMap::from([
        ("session", diagnostics(session_diagnostics, &scopes)),
        ("main", diagnostics(main_diagnostics, &scopes)),
    ]);
    if native {
        graphs.insert("process", diagnostics(process_rows(agent.root()), &scopes));
    }
    let mermaid = mermaid(&graphs);
    let mut out = root().join("docs/architecture/rust-agent/generated");
    if native {
        out = out.join("aimux");
    }
    std::fs::create_dir_all(&out)?;
    std::fs::write(
        out.join("services.json"),
        serde_json::to_string_pretty(&graphs)? + "\n",
    )?;
    std::fs::write(out.join("services.mmd"), &mermaid)?;
    let command = if native {
        "cargo run -p xtask -- graph --model-config crates/dim-agent-aimux/examples/offline.json"
    } else {
        "cargo run -p xtask -- graph"
    };
    std::fs::write(
        out.join("services.md"),
        format!(
            "# 默认运行组合\n\n由真实 rutis 装配生成。重新生成：`{command}`。\n\n```mermaid\n{mermaid}```\n"
        ),
    )?;
    if !native {
        let branch = session
            .branch(
                "main".into(),
                0,
                BranchOptions {
                    prompt: "diagram".into(),
                    source: dim_agent_types::BranchSource::Host {
                        operation_id: "diagram".into(),
                    },
                },
            )
            .await?;
        session.idle(&branch).await?;
    }
    instances(&session, &out)?;
    agent.close().await?;
    if !native {
        plugins::write(&root(), &plugins::local_graph().await?)?;
        app_server::write(&root(), app_server::graph().await?)?;
        catalog::write(&root()).await?;
    }
    boundaries(true)?;
    println!("generated {}", out.display());
    Ok(())
}
/// `graph` writes the generated architecture files; `check` renders them again and fails when the committed bytes differ.
fn boundaries(write: bool) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root())
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    let mut edges = vec![];
    let mut graph = String::from("%% Generated from cargo metadata; do not edit.\nflowchart LR\n");
    for p in metadata["packages"].as_array().unwrap() {
        let name = p["name"].as_str().unwrap();
        for d in p["dependencies"].as_array().unwrap() {
            let dep = d["name"].as_str().unwrap();
            if name == "dim-agent-types" && !modules::TYPES_VALUE_DEPENDENCIES.contains(&dep) {
                return Err("types must not depend on runtime/capabilities".into());
            }
            if name == "dim-agent-runtime"
                && (dep == "dim-agent-capabilities" || dep == "dim-app-server")
            {
                return Err("runtime must not depend on capabilities/server".into());
            }
            if name == "dim-agent-capabilities" && dep == "dim-app-server" {
                return Err("capabilities must not depend on server".into());
            }
            if dep.starts_with("dim-") || dep == "rutis" {
                edges.push(json!({"from":name,"to":dep}));
                graph += &format!(
                    "  {}[\"{name}\"] --> {}[\"{dep}\"]\n",
                    name.replace('-', "_"),
                    dep.replace('-', "_")
                );
            }
        }
    }
    let out = root().join("docs/architecture/rust-agent/generated");
    let mut files = vec![
        ("crates.json", serde_json::to_string_pretty(&edges)? + "\n"),
        ("crates.mmd", graph),
    ];
    files.extend(modules::generate(&root())?);
    if write {
        std::fs::create_dir_all(&out)?;
        for (name, text) in &files {
            std::fs::write(out.join(name), text)?;
        }
    } else {
        let stale: Vec<_> = files
            .iter()
            .filter(|(name, text)| {
                std::fs::read_to_string(out.join(name)).ok().as_ref() != Some(text)
            })
            .map(|(name, _)| out.join(name).display().to_string())
            .collect();
        if !stale.is_empty() {
            return Err(format!(
                "generated architecture files are stale: {}; run `cargo run -p xtask -- graph`",
                stale.join(", ")
            )
            .into());
        }
    }
    println!("crate and module boundaries passed");
    plugins::check(&root())?;
    Ok(())
}
async fn measure() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let before = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (agent, config, native) = configured_agent().await?;
    let mut runs = vec![];
    for count in [1usize, 10, 100, 1000] {
        let bytes_before = allocation::live();
        let start = Instant::now();
        let mut sessions = vec![];
        for _ in 0..count {
            sessions.push(agent.create_session(config.clone()).await?);
        }
        let create_ms = start.elapsed().as_secs_f64() * 1000.;
        let bytes_active = allocation::live();
        let active = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        let start = Instant::now();
        for session in &sessions {
            session.close().await?;
        }
        drop(sessions);
        tokio::task::yield_now().await;
        runs.push(json!({"sessions":count,"allocationBytesBefore":bytes_before,"allocationBytesActive":bytes_active,"allocationBytesAfterClose":allocation::live(),"createMs":create_ms,"closeMs":start.elapsed().as_secs_f64()*1000.,"activeTasks":active,"tasksAfterClose":tokio::runtime::Handle::current().metrics().num_alive_tasks()}));
    }
    let mut concurrent = Vec::new();
    for count in [1usize, 16, 64].into_iter().filter(|_| !native) {
        let session = agent.create_session(config.clone()).await?;
        let mut branches = vec!["main".to_owned()];
        for index in 1..count {
            branches.push(
                session
                    .branch(
                        "main".into(),
                        0,
                        BranchOptions {
                            prompt: "warmup".into(),
                            source: dim_agent_types::BranchSource::Host {
                                operation_id: format!("measure-{index}"),
                            },
                        },
                    )
                    .await?,
            );
        }
        for branch in &branches {
            session.idle(branch).await?;
        }
        let mut jobs = tokio::task::JoinSet::new();
        let wall = Instant::now();
        for branch in branches {
            let session = session.clone();
            jobs.spawn(async move {
                let start = Instant::now();
                session
                    .submit(
                        "measurement".into(),
                        dim_agent_types::InputMode::Queue,
                        &branch,
                    )
                    .await?;
                session.idle(&branch).await?;
                Ok::<_, AgentError>(start.elapsed().as_secs_f64() * 1000.)
            });
        }
        let mut latency = Vec::new();
        while let Some(result) = jobs.join_next().await {
            latency.push(result??);
        }
        latency.sort_by(f64::total_cmp);
        let percentile =
            |p: f64| latency[((latency.len() as f64 * p).ceil() as usize).saturating_sub(1)];
        concurrent.push(json!({"activeBranches":count,"wallMs":wall.elapsed().as_secs_f64()*1000.,"p50Ms":percentile(0.50),"p95Ms":percentile(0.95),"p99Ms":percentile(0.99)}));
        session.close().await?;
    }
    agent.close().await?;
    let settling = Instant::now();
    while tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
        > before
        && settling.elapsed() < std::time::Duration::from_secs(30)
    {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let report = json!({"modelHost": if native { "aimux" } else { "mock" }, "debugAssertions":cfg!(debug_assertions),"runtimeWorkers":2,"taskBaseline":before,"cleanupSettleMs":settling.elapsed().as_secs_f64()*1000.,"tasksAfterAgentClose":tokio::runtime::Handle::current().metrics().num_alive_tasks(),"allocationBytesAfterAgentClose":allocation::live(),"peakRustAllocationBytes":allocation::peak(),"runs":runs,"concurrentMockTurns":concurrent,"agentEventBytes":std::mem::size_of::<dim_agent_types::AgentEvent>(),"modelRequestBytes":std::mem::size_of::<dim_agent_types::ModelRequest>()});
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn instances(
    session: &Session,
    out: &std::path::Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut branches = session.branches();
    branches.sort_by_key(|branch| (branch.parent.is_some(), branch.id.clone()));
    let aliases: BTreeMap<_, _> = branches
        .iter()
        .enumerate()
        .map(|(index, b)| (b.id.clone(), format!("timeline_{index}")))
        .collect();
    let mut graph = String::from(
        "%% Generated from live Session/Timeline registrations. Random identities are aliased.\nflowchart TB\n  agent[Agent] --> session[Session]\n",
    );
    let mut nodes = Vec::new();
    for branch in branches {
        let alias = &aliases[&branch.id];
        let label = if branch.parent.is_none() {
            "main".to_owned()
        } else {
            alias.clone()
        };
        let status = session.state(&branch.id)?;
        let phase = session
            .get::<LoopSnapshot>(&branch.id)?
            .map_or(LoopPhase::Idle, |s| s.phase);
        let queued = session
            .get::<InputSnapshot>(&branch.id)?
            .map_or(0, |s| s.queued);
        let parent = branch
            .parent
            .as_ref()
            .and_then(|id| aliases.get(id))
            .cloned();
        graph +=
            &format!("  session --> {alias}[\"{label} / phase={phase:?} / queued={queued}\"]\n");
        if let Some(parent) = &parent {
            graph += &format!("  {parent} -.->|branch| {alias}\n");
        }
        nodes.push(json!({"alias":alias,"parent":parent,"branchPoint":branch.branch_point,"createAnchor":branch.create_anchor,"status":status,"phase":phase,"queued":queued,"pluginDrivers":session.branch_diagnostics(&branch.id)?.plugins.len()}));
    }
    std::fs::write(
        out.join("instances.json"),
        serde_json::to_string_pretty(
            &json!({"source":"Session branches/state and typed capability snapshots","identities":"stable aliases for generated random IDs","timelines":nodes}),
        )? + "\n",
    )?;
    std::fs::write(out.join("instances.mmd"), graph)?;
    Ok(())
}
