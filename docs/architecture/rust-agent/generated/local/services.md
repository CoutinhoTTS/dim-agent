# 本地执行运行组合

由真实 rutis 装配 `defaults::install_local_execution` 生成（模型为脚本化 mock）。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p25["branch"]
    p42["SkillsBranch"]
    p41["PromptSourcesBranch"]
    p40["WaitBranch"]
    p39["BackgroundExec"]
    p38["FileReads"]
    p37["ToolExecutor"]
    p36["ToolBinding"]
    p35["Approval"]
    p34["Permission"]
    p33["ModelProvider"]
    p32["Loop"]
    p31["InputQueue"]
    p30["Prompt"]
    p29["ContextPreparation"]
    p28["TreeTimeline"]
    p27["SessionBroadcast"]
    p26["BranchScope"]
  end
  subgraph process["process"]
    p6["rutis-loader"]
    p5["FileWriteCoordinator"]
    p4["SkillsCatalog"]
    p3["HostPlatform"]
    p2["FileStorage"]
  end
  subgraph session["session"]
    p9["session"]
    p23["SkillContentInstances"]
    p24["SkillContent"]
    p22["Skills"]
    p21["SkillContributions"]
    p20["PromptSources"]
    p19["PromptRegistry"]
    p18["Wait"]
    p17["WorkspaceTools"]
    p16["ToolRegistry"]
    p15["InputMethods"]
    p14["TimelineTree"]
    p13["SessionBroadcastEnd"]
    p12["PrepareModelHost"]
    p11["SessionExtension"]
    p10["SessionScope"]
  end
  p22 -->|"dim_agent_capabilities::skills::session::SessionSkills@session"| p42
  p26 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p42
  p30 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p42
  p19 -->|"dim_agent_capabilities::host::PromptHost@session"| p41
  p26 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p41
  p30 -->|"dim_agent_runtime::state::StateCell<dim_agent_capabilities::prompt_state::PromptSnapshot>@main"| p41
  p30 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p41
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p40
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p40
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p39
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p39
  p35 -->|"dim_agent_capabilities::approval::Approval@main"| p37
  p36 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p37
  p26 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p37
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p37
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p37
  p26 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p37
  p30 -->|"dim_agent_capabilities::prompt::Prompt@main"| p36
  p16 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p36
  p26 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p36
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p36
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p36
  p26 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p36
  p34 -->|"dim_agent_capabilities::permission::Permission@main"| p35
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p35
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p35
  p10 -->|"dim_agent_capabilities::permission::CommandRulesHost@session"| p34
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p34
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p34
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p34
  p29 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p33
  p10 -->|"dim_agent_capabilities::host::ModelHost@session"| p33
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p33
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p33
  p26 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p33
  p10 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p32
  p10 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p32
  p10 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p32
  p12 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p32
  p36 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p32
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p32
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p32
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p31
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p31
  p29 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p30
  p19 -->|"dim_agent_capabilities::host::PromptHost@session"| p30
  p19 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p30
  p26 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p30
  p26 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p30
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p30
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p30
  p26 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p30
  p10 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p30
  p26 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p30
  p10 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p29
  p10 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p29
  p10 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p29
  p10 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p29
  p26 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p29
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p29
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p29
  p26 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p29
  p14 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p28
  p26 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p28
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p28
  p26 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p28
  p26 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p27
  p26 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p27
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p4
  p2 -->|"dim_agent_capabilities::file_storage::FileStorage"| p3
  p21 -->|"dim_agent_capabilities::skills::content::Contributions@session"| p24
  p4 -->|"dim_agent_capabilities::skills::catalog::SkillsCatalog"| p22
  p21 -->|"dim_agent_capabilities::skills::content::Contributions@session"| p22
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p22
  p10 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p22
  p10 -->|"dim_agent_runtime::scope::SessionLoader@session"| p22
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p20
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p20
  p20 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/base@session"| p19
  p20 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/environment/context@session"| p19
  p20 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/environment/date@session"| p19
  p20 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/instructions/project@session"| p19
  p20 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/instructions/user@session"| p19
  p20 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/session@session"| p19
  p22 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/skills/instructions@session"| p19
  p22 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/skills/list@session"| p19
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p19
  p14 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p18
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p17
  p14 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p17
  p5 -->|"dim_agent_capabilities::workspace_tools::files::FileWriteCoordinator"| p17
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p17
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p17
  p17 -->|"dim_agent_capabilities::host::ToolHost@session"| p16
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p16
  p10 -->|"dim_agent_runtime::diagnostics::Diagnostics@session"| p16
  p14 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p15
  p10 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p14
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p14
  p10 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p14
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p13
  p16 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p12
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p12
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p11
```
