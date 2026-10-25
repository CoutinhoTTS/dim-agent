use crate::{
    adapters::*,
    agent_loop::LoopPlugin,
    approval::ApprovalPlugin,
    context_preparation::ContextPreparationPlugin,
    history::HistoryRenderer,
    host::*,
    input_queue::InputQueuePlugin,
    model_provider::ModelProviderPlugin,
    permission::{CommandRulesHost, NoCommandRules, PermissionPlugin},
    prompt::{InitialPromptConfiguration, PromptPlugin},
    session_broadcast::{SessionBroadcastEndPlugin, SessionBroadcastPlugin},
    timeline_tree::{TimelineTreePlugin, TreeTimelinePlugin},
    tool_binding::ToolBindingPlugin,
    tool_executor::ToolExecutorPlugin,
    tool_registry::ToolRegistryPlugin,
};
use dim_agent_runtime::{AgentBuilder, Names};
use rutis::CordisError;
use std::sync::Arc;

/// The same timeline factories apply to main and every future branch.
pub fn install(builder: &mut AgentBuilder) {
    install_with_prompt(builder, Arc::new(EmptyPromptHost), None);
}

pub fn install_with_prompt(
    builder: &mut AgentBuilder,
    host: Arc<dyn LoadPrompt>,
    initial: Option<dim_agent_runtime::ConfigureUpdate>,
) {
    install_profile(builder, host, initial, ModelProviderPlugin::new);
}

/// Install the same execution loop with process-owned network model services.
pub fn install_with_process_model(builder: &mut AgentBuilder) {
    install_profile(
        builder,
        Arc::new(EmptyPromptHost),
        None,
        ModelProviderPlugin::process,
    );
}

fn install_profile(
    builder: &mut AgentBuilder,
    host: Arc<dyn LoadPrompt>,
    initial: Option<dim_agent_runtime::ConfigureUpdate>,
    model: fn(&Names<'_>) -> Result<ModelProviderPlugin, CordisError>,
) {
    builder
        .session_value(Arc::new(PromptHost(host)))
        .session_value(Arc::new(InitialPromptConfiguration(initial)))
        .session_value(Arc::new(CommandRulesHost(Arc::new(NoCommandRules))));
    install_with_prompt_capability(builder, model);
}

/// Installs the same capabilities without binding `PromptHost` or
/// `InitialPromptConfiguration`: a Session capability such as PromptSources
/// provides both. The host binds `CommandRulesHost`.
pub fn install_with_prompt_capability(
    builder: &mut AgentBuilder,
    model: fn(&Names<'_>) -> Result<ModelProviderPlugin, CordisError>,
) {
    install_with_prompt_capability_and_branch(builder, model, true);
}

/// Base variant without the branch tool; used by the minimal execution
/// assembly where the branch tool is not part of the contract.
pub fn install_with_prompt_capability_no_branch(
    builder: &mut AgentBuilder,
    model: fn(&Names<'_>) -> Result<ModelProviderPlugin, CordisError>,
) {
    install_with_prompt_capability_and_branch(builder, model, false);
}

fn install_with_prompt_capability_and_branch(
    builder: &mut AgentBuilder,
    model: fn(&Names<'_>) -> Result<ModelProviderPlugin, CordisError>,
    branch_tool: bool,
) {
    crate::register_names(builder);
    builder
        .session_part(|names| names.part(SessionBroadcastEndPlugin))
        .session_part(|names| names.part(TimelineTreePlugin::new(names)?))
        .session_part(|names| names.part(crate::input_queue::InputMethodsPlugin::new(names)?))
        .session_methods(crate::input_queue::INPUT_METHODS)
        .session_tool_registry(ToolRegistryPlugin::new)
        .branch_part(|names| names.part(SessionBroadcastPlugin))
        .branch_part(|names| names.part(TreeTimelinePlugin))
        .branch_part(|names| names.part(ContextPreparationPlugin::new(names)?))
        .branch_part(|names| names.part(PromptPlugin::new(names)?))
        .branch_part(|names| names.part(InputQueuePlugin::new(names)?))
        .branch_part(|names| names.part(LoopPlugin::new(names)?))
        .branch_part(move |names| model(names)?.part(names))
        .branch_part(|names| names.part(PermissionPlugin::new(names)?))
        .branch_part(|names| names.part(ApprovalPlugin::new(names)?))
        .branch_part(|names| names.part(ToolBindingPlugin::new(names)?))
        .branch_part(|names| names.part(ToolExecutorPlugin::new(names)?));
    if branch_tool {
        builder.session_tool("branch");
    }
}

/// Product profile of the minimal execution assembly: only `execute` as the
/// local tool, with Skills and prompt sources. No workspace tools, no wait,
/// no branch tool. The host binds `CommandRulesHost`. See
/// `docs/design/appserver/minimal-execution.md`.
pub fn install_minimal_execution(
    builder: &mut AgentBuilder,
    model: fn(&Names<'_>) -> Result<ModelProviderPlugin, CordisError>,
) {
    install_with_prompt_capability_no_branch(builder, model);
    builder
        .branch_part(|names| names.part(crate::shell_session::ShellSessionPlugin::session(names)?));
    crate::shell_tools::install(builder);
    crate::prompt_sources::install(builder);
    crate::skills::install(builder);
}

/// Product profile of the local execution and context capabilities. The
/// composition root also registers the process plugins `HostPlatformPlugin`,
/// `FileWriteCoordinatorPlugin`, and `SkillsCatalogPlugin`, plus the product
/// `SkillSettingsHost`, and binds the Session `CommandRulesHost`. A new
/// Session adopts `SessionConfig.model` with its prompt sources.
pub fn install_local_execution(
    builder: &mut AgentBuilder,
    model: fn(&Names<'_>) -> Result<ModelProviderPlugin, CordisError>,
    workspace: crate::workspace_tools::WorkspaceToolsConfig,
) {
    install_with_prompt_capability(builder, model);
    crate::workspace_tools::install(builder, workspace);
    crate::wait::install(builder);
    crate::shell_input::install(builder);
    crate::prompt_sources::install(builder);
    crate::skills::install(builder);
}

/// Installs `driver` as the Session `ToolHost` and records its tool names.
pub fn tool_host(builder: &mut AgentBuilder, driver: Arc<dyn ToolDriver>) {
    crate::register_names(builder);
    builder
        .host_tools(driver.definitions().into_iter().map(|tool| tool.name))
        .session_value(Arc::new(ToolHost(driver)));
}

/// Use this with a custom `ToolHost` binding, e.g. [`tool_host`].
pub fn mock_model_hosts(builder: &mut AgentBuilder) {
    mock_model_hosts_with_history(builder, HistoryRenderer::default());
}

/// Configure a pure history renderer without installing a duplicate binding.
pub fn mock_model_hosts_with_history(builder: &mut AgentBuilder, history: HistoryRenderer) {
    crate::register_names(builder);
    builder
        .session_value(Arc::new(history))
        .process_value(Arc::new(ModelHost(Arc::new(EchoModel))))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_value(Arc::new(CompactionStrategyHost(None)))
        .session_part(|names| names.part(DefaultPrepareModelPlugin::new(names)?));
}
pub fn mock_hosts(builder: &mut AgentBuilder) {
    mock_model_hosts(builder);
    tool_host(builder, Arc::new(MockTools));
}
