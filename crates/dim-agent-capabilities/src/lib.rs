pub mod adapters;
pub mod agent_loop;
pub mod approval;
pub(crate) mod branch_input;
pub mod context_block;
pub mod context_methods;
pub mod context_model;
mod context_policy;
pub mod context_preparation;
pub mod defaults;
pub mod exec_stop;
pub mod file_storage;
pub mod history;
pub mod host;
pub mod host_platform;
pub mod input_queue;
pub mod methods;
pub mod model_provider;
mod output;
pub mod permission;
pub mod prompt;
pub mod prompt_registry;
pub mod prompt_sources;
pub mod prompt_state;
pub mod session_broadcast;
pub mod settings;
pub mod shell_input;
pub mod shell_session;
pub mod shell_tools;
pub mod skill_settings;
pub mod skills;
pub mod steer;
pub mod summary_strategy;
pub mod timeline_tree;
pub mod tool_binding;
pub mod tool_executor;
pub mod tool_registry;
pub mod wait;
pub mod workspace_tools;

use dim_agent_runtime::{BranchService, PublicService, SessionService};
/// Registers the names of this crate's Session and branch services (plugin
/// contract §4). Every installer below applies it; registering twice does
/// nothing.
pub fn register_names(builder: &mut dim_agent_runtime::AgentBuilder) {
    use dim_agent_runtime::{BRANCH_GROUP, SESSION_GROUP, names::register};
    let catalog = builder.catalog_mut();
    let session = Some(SESSION_GROUP);
    register::<timeline_tree::TimelineTree>(catalog, session);
    register::<timeline_tree::TreeCommands>(catalog, session);
    register::<tool_registry::ToolRegistry>(catalog, session);
    register::<tool_registry::ToolSourcesView>(catalog, session);
    register::<history::HistoryRenderer>(catalog, session);
    register::<prompt::InitialPromptConfiguration>(catalog, session);
    register::<skills::SessionSkills>(catalog, session);
    register::<host::PrepareModelHost>(catalog, session);
    register::<host::PrepareContextHost>(catalog, session);
    register::<host::ContextModelHost>(catalog, session);
    register::<host::CompactionStrategyHost>(catalog, session);
    register::<host::PromptHost>(catalog, session);
    register::<host::ModelHost>(catalog, session);
    register::<host::ToolHost>(catalog, session);
    register::<permission::CommandRulesHost>(catalog, session);
    register::<methods::SessionMethods>(catalog, session);
    let branch = Some(BRANCH_GROUP);
    register::<input_queue::InputQueue>(catalog, branch);
    register::<agent_loop::AgentLoop>(catalog, branch);
    register::<prompt::Prompt>(catalog, branch);
    register::<context_preparation::ContextPreparation>(catalog, branch);
    register::<context_preparation::ConfigurationPublisher>(catalog, branch);
    register::<model_provider::ModelProvider>(catalog, branch);
    register::<tool_executor::ToolExecutor>(catalog, branch);
    register::<tool_binding::ToolBinding>(catalog, branch);
    register::<permission::Permission>(catalog, branch);
    register::<approval::Approval>(catalog, branch);
    register::<skills::SkillsBranch>(catalog, branch);
    register::<workspace_tools::background::BackgroundExec>(catalog, branch);
    register::<shell_session::ShellSession>(catalog, branch);
    register::<dyn exec_stop::ExecStop>(catalog, branch);
    register::<wait::Wait>(catalog, branch);
}

impl SessionService for timeline_tree::TimelineTree {}
impl SessionService for timeline_tree::TreeCommands {}
impl SessionService for tool_registry::ToolRegistry {}
impl SessionService for history::HistoryRenderer {}
impl SessionService for prompt::InitialPromptConfiguration {}
impl SessionService for prompt_registry::PromptSource {}
impl SessionService for skills::SessionSkills {}
impl SessionService for host::PrepareModelHost {}
impl SessionService for host::PrepareContextHost {}
impl SessionService for host::ContextModelHost {}
impl SessionService for host::CompactionStrategyHost {}
impl SessionService for host::PromptHost {}
impl SessionService for permission::CommandRulesHost {}
impl SessionService for host::ModelHost {}
impl SessionService for host::ToolHost {}
impl SessionService for methods::SessionMethods {}
impl BranchService for input_queue::InputQueue {}
impl BranchService for agent_loop::AgentLoop {}
impl BranchService for prompt::Prompt {}
impl BranchService for context_preparation::ContextPreparation {}
impl BranchService for context_preparation::ConfigurationPublisher {}
impl BranchService for model_provider::ModelProvider {}
impl BranchService for tool_executor::ToolExecutor {}
impl BranchService for tool_binding::ToolBinding {}
impl BranchService for permission::Permission {}
impl BranchService for approval::Approval {}
impl BranchService for skills::SkillsBranch {}
impl BranchService for workspace_tools::background::BackgroundExec {}
impl BranchService for shell_session::ShellSession {}
impl BranchService for dyn exec_stop::ExecStop {}
impl BranchService for wait::Wait {}

// Business ports safe to obtain from a Session handle: RPC calls them by
// request branch, tool handlers by call branch.
impl PublicService for approval::Approval {}
impl PublicService for input_queue::InputQueue {}
impl PublicService for permission::Permission {}
impl PublicService for workspace_tools::background::BackgroundExec {}
impl PublicService for shell_session::ShellSession {}
impl PublicService for dyn exec_stop::ExecStop {}
impl PublicService for workspace_tools::file_reads::FileReads {}
impl PublicService for wait::Wait {}
impl PublicService for skills::SkillsBranch {}
impl PublicService for prompt::Prompt {}
impl PublicService for tool_registry::ToolSourcesView {}
impl PublicService for methods::SessionMethods {}
impl SessionService for tool_registry::ToolSourcesView {}
impl PublicService for agent_loop::AgentLoop {}
impl PublicService for context_preparation::ContextPreparation {}
