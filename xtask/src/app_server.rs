//! `graph` assembles the App Server product profile through `serve` and
//! records its process plugins in `generated/app-server/`.
use dim_agent_capabilities::{
    adapters::DefaultPrepareModelPlugin,
    context_methods::{self, ContextMethodsPlugin},
    context_model::ProductContextModel,
    defaults,
    history::HistoryRenderer,
    host::{ContextModelHost, IdentityContext, PrepareContextHost},
    model_provider::ModelProviderPlugin,
    summary_strategy::ProductSummaryStrategyPlugin,
    workspace_tools::WorkspaceToolsConfig,
};
use dim_agent_runtime::{AgentBuilder, AgentConfig};
use dim_app_server::{RunningServer, ServerConfig, serve};
use rutis::RuntimeDiagnostics;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A scratch directory for one profile, removed by the caller.
pub fn scratch(label: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("dim-xtask-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir.canonicalize()?)
}

/// Serves the product profile in `dir` with the Agent assembly
/// `dim-app-server --product-home` installs (`cli.rs`), after `observe` adds
/// what the caller observes it with.
pub async fn serve_product(
    dir: &Path,
    observe: impl FnOnce(&mut AgentBuilder),
) -> Result<RunningServer> {
    let mut config = ServerConfig {
        websocket_address: Some(([127, 0, 0, 1], 0).into()),
        product_home: Some(dir.join("home")),
        target_home: Some(dir.join("rust")),
        ..Default::default()
    };
    config.session_defaults.cwd = dir.to_string_lossy().into_owned();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::install_local_execution(
        &mut builder,
        ModelProviderPlugin::process,
        WorkspaceToolsConfig {
            output_root: dir.join("command-output"),
        },
    );
    dim_app_server::permission_rules::install(&mut builder);
    builder
        .session_value(Arc::new(HistoryRenderer::default()))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(Some(Arc::new(
            ProductContextModel,
        )))))
        .session_part(|names| names.part(ProductSummaryStrategyPlugin::new(names)?))
        .session_part(|names| names.part(ContextMethodsPlugin::new(names)?))
        .session_methods(context_methods::NAMESPACE)
        .session_part(|names| names.part(DefaultPrepareModelPlugin::new(names)?));
    dim_app_server::collaboration::install(&mut builder);
    dim_agent_questions::install(&mut builder);
    observe(&mut builder);
    Ok(serve(builder, config).await?)
}

/// Serves the product profile and returns the process layer: every plugin
/// outside the Agent instances.
pub async fn graph() -> Result<Value> {
    let dir = scratch("server")?;
    let server = serve_product(&dir, |_| {}).await?;
    let snapshot = process_layer(server.diagnostics());
    let graph = crate::diagnostics(snapshot, &[]);
    server.shutdown();
    server.join().await?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(graph)
}

/// The process layer of a served profile: every plugin outside the Agent
/// instances, and the services they provide.
pub fn process_layer(mut snapshot: RuntimeDiagnostics) -> RuntimeDiagnostics {
    // An Agent instance is everything under the `AgentScope` plugin's parent.
    let parents: BTreeMap<_, _> = snapshot
        .plugins
        .iter()
        .map(|plugin| (plugin.id, plugin.parent))
        .collect();
    let agents: Vec<_> = snapshot
        .plugins
        .iter()
        .filter(|plugin| plugin.name == "AgentScope")
        .filter_map(|plugin| plugin.parent)
        .collect();
    let inside = |mut id| loop {
        if agents.contains(&id) {
            return true;
        }
        match parents.get(&id).copied().flatten() {
            Some(parent) => id = parent,
            None => return false,
        }
    };
    snapshot.plugins.retain(|plugin| !inside(plugin.id));
    let members: Vec<_> = snapshot.plugins.iter().map(|plugin| plugin.id).collect();
    snapshot
        .bindings
        .retain(|binding| members.contains(&binding.provider));
    snapshot
}

/// Writes the process graph of the product profile.
pub fn write(root: &Path, graph: Value) -> Result<()> {
    let out = root.join("docs/architecture/rust-agent/generated/app-server");
    std::fs::create_dir_all(&out)?;
    let graphs = BTreeMap::from([("process", graph)]);
    std::fs::write(
        out.join("services.json"),
        serde_json::to_string_pretty(&graphs)? + "\n",
    )?;
    let mermaid = crate::mermaid(&graphs);
    std::fs::write(out.join("services.mmd"), &mermaid)?;
    std::fs::write(
        out.join("services.md"),
        format!(
            "# App Server 进程组合\n\n由 `serve` 以产品配置（`--product-home`）真实装配生成，只含进程层，不含 Agent 实例。重新生成：`cargo run -p xtask -- graph`。\n\n```mermaid\n{mermaid}```\n"
        ),
    )?;
    Ok(())
}
