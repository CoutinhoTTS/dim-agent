//! `graph` assembles the App Server product profile through `serve` and
//! records its process plugins in `generated/app-server/`.
use dim_agent_capabilities::{
    adapters::DefaultPrepareModelPlugin,
    context_model::ProductContextModel,
    defaults,
    history::HistoryRenderer,
    host::{IdentityContext, PrepareContextHost},
    model_provider::ModelProviderPlugin,
    summary_strategy::ProductSummaryStrategyConfig,
    workspace_tools::WorkspaceToolsConfig,
};
use dim_agent_runtime::{AgentBuilder, AgentConfig, Feature};
use dim_agent_types::ModelSelection;
use dim_app_server::{RunningServer, ServerConfig, serve};
use rutis::host::diagnostics::RuntimeDiagnostics;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The candidate texts that replace their factory configuration members in
/// the assembly [`serve_home`] builds (model-prompts §8.1). The product never
/// sets a member here.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    /// The summary strategy's system prompt in place of its built-in
    /// `SYSTEM` (ContextCompaction §13.5).
    pub summary_prompt: Option<Arc<str>>,
}

/// What one run adds to the product assembly: the overrides above, the model
/// configuration file `dim-app-server --model-config` serves, and the local
/// endpoint a client connects to. The product path leaves every field empty.
#[derive(Clone, Default)]
pub struct RunSettings {
    pub overrides: Overrides,
    pub model_config: Option<PathBuf>,
    pub unix_path: Option<PathBuf>,
}

impl RunSettings {
    /// The native store the served profile keeps under `home`: `home/rust`
    /// holds it in the product layout, and a model configuration's profile
    /// (`dim-app-server --model-config`) keeps it in `home` itself.
    pub fn database(&self, home: &Path) -> PathBuf {
        if self.model_config.is_some() {
            home.join("dimcode.sqlite")
        } else {
            home.join("rust").join("dimcode.sqlite")
        }
    }
}

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
    serve_home(
        &dir.join("home"),
        dir,
        dir.join("command-output"),
        RunSettings::default(),
        observe,
    )
    .await
}

/// [`serve_product`] on an existing product home: its native store stays at
/// `home/rust`, the layout `dimcode` starts the daemon with. `run` selects the
/// model profile and the overrides a run adds; the product path passes
/// [`RunSettings::default`].
pub async fn serve_home(
    home: &Path,
    cwd: &Path,
    output_root: PathBuf,
    run: RunSettings,
    observe: impl FnOnce(&mut AgentBuilder),
) -> Result<RunningServer> {
    let home = home.to_path_buf();
    let product = run.model_config.is_none();
    let mut config = ServerConfig {
        websocket_address: Some(([127, 0, 0, 1], 0).into()),
        unix_path: run.unix_path.clone(),
        product_home: product.then(|| home.clone()),
        target_home: Some(if product {
            home.join("rust")
        } else {
            home.clone()
        }),
        ..Default::default()
    };
    config.session_defaults.cwd = cwd.to_string_lossy().into_owned();
    let summary = ProductSummaryStrategyConfig {
        system_prompt: run.overrides.summary_prompt.clone(),
        ..Default::default()
    };
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::install_local_execution(
        &mut builder,
        ModelProviderPlugin::process,
        WorkspaceToolsConfig { output_root },
    );
    dim_app_server::permission_rules::install(&mut builder);
    builder
        .session_value(Arc::new(HistoryRenderer::default()))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))));
    match run.model_config {
        Some(path) => {
            let (models, profile) = dim_app_server::cli::model_profile(&path)?;
            config.daemon_profile = profile;
            config.model_catalog = Some(models.clone());
            config.session_defaults.model = ModelSelection {
                provider_id: models.default_model().provider_id.clone(),
                model_id: models.default_model().model_id.clone(),
                capability_snapshot: None,
                config: None,
                metadata: None,
            };
            defaults::install_compaction(&mut builder, models.clone(), summary);
            dim_agent_aimux::install(&mut builder);
        }
        None => {
            defaults::install_compaction(&mut builder, Arc::new(ProductContextModel), summary);
            builder.require_feature(
                Feature::new("PrepareModelHost").session("PrepareModelHost", |names| {
                    Ok(Box::new(DefaultPrepareModelPlugin::new(names)?))
                }),
            );
        }
    }
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
