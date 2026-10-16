//! `graph` assembles the App Server product profile through `serve` and
//! records its process plugins in `generated/app-server/`.
use dim_agent_capabilities::{
    adapters::DefaultPrepareModelPlugin,
    defaults,
    history::HistoryRenderer,
    host::{CompactionStrategyHost, ContextModelHost, IdentityContext, PrepareContextHost},
    model_provider::ModelProviderPlugin,
    workspace_tools::WorkspaceToolsConfig,
};
use dim_agent_runtime::{AgentBuilder, AgentConfig};
use dim_app_server::{ServerConfig, serve};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, sync::Arc};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Serves the product profile as `dim-app-server --product-home` does and
/// returns the process layer: every plugin outside the Agent instances.
pub async fn graph() -> Result<Value> {
    let dir = std::env::temp_dir().join(format!("dim-xtask-server-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let dir = dir.canonicalize()?;
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
    builder
        .session_value(Arc::new(HistoryRenderer::default()))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_value(Arc::new(CompactionStrategyHost(None)))
        .session_plugin(DefaultPrepareModelPlugin::new);
    let server = serve(builder, config).await?;
    let mut snapshot = server.diagnostics();
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
    let graph = crate::diagnostics(snapshot, &[]);
    server.shutdown();
    server.join().await?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(graph)
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
