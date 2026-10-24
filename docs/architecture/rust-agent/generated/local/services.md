# 本地执行运行组合

由真实 rutis 装配 `defaults::install_local_execution` 生成（模型为脚本化 mock）。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p28["branch"]
    p45["SkillsBranch"]
    p44["ToolExecutor"]
    p43["ToolBinding"]
    p42["ModelProvider"]
    p41["Loop"]
    p40["Prompt"]
    p39["ContextPreparation"]
    p38["SessionBroadcast"]
    p37["PromptSourcesBranch"]
    p36["WaitBranch"]
    p35["BackgroundExec"]
    p34["FileReads"]
    p33["Approval"]
    p32["Permission"]
    p31["InputQueue"]
    p30["TreeTimeline"]
    p29["BranchScope"]
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
    p26["SkillContentInstances"]
    p27["SkillContent"]
    p25["PromptSources"]
    p24["Wait"]
    p23["InputMethods"]
    p22["Questions"]
    p21["Attachments"]
    p20["Skills"]
    p19["SkillContributions"]
    p18["PromptRegistry"]
    p17["ShellInput"]
    p16["WorkspaceTools"]
    p15["ToolRegistry"]
    p14["TimelineTree"]
    p13["SessionBroadcastEnd"]
    p12["PrepareModelHost"]
    p11["SessionExtension"]
    p10["SessionScope"]
  end
  p20 -->|"dim_agent_capabilities::skills::session::SessionSkills@session"| p45
  p29 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p45
  p40 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p45
  p33 -->|"dim_agent_capabilities::approval::Approval@main"| p44
  p43 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p44
  p29 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p44
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p44
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p44
  p40 -->|"dim_agent_capabilities::prompt::Prompt@main"| p43
  p15 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p43
  p29 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p43
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p43
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p43
  p29 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p43
  p39 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p42
  p10 -->|"dim_agent_capabilities::host::ModelHost@session"| p42
  p29 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p42
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p42
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p42
  p10 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p41
  p10 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p41
  p10 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p41
  p12 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p41
  p43 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p41
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p41
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p41
  p39 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p40
  p18 -->|"dim_agent_capabilities::host::PromptHost@session"| p40
  p18 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p40
  p29 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p40
  p29 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p40
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p40
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p40
  p10 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p40
  p29 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p40
  p10 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p39
  p10 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p39
  p10 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p39
  p10 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p39
  p29 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p39
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p39
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p39
  p29 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p38
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p38
  p18 -->|"dim_agent_capabilities::host::PromptHost@session"| p37
  p29 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p37
  p40 -->|"dim_agent_runtime::state::StateCell<dim_agent_capabilities::prompt_state::PromptSnapshot>@main"| p37
  p40 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p37
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p36
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p36
  p10 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p35
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p35
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p35
  p32 -->|"dim_agent_capabilities::permission::Permission@main"| p33
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p33
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p33
  p10 -->|"dim_agent_capabilities::permission::CommandRulesHost@session"| p32
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p32
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p32
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p32
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p31
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p31
  p14 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p30
  p29 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p30
  p29 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p30
  p29 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p30
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p4
  p2 -->|"dim_agent_capabilities::file_storage::FileStorage"| p3
  p19 -->|"dim_agent_capabilities::skills::content::Contributions@session"| p27
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p25
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p25
  p14 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p24
  p14 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p23
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p22
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p21
  p4 -->|"dim_agent_capabilities::skills::catalog::SkillsCatalog"| p20
  p19 -->|"dim_agent_capabilities::skills::content::Contributions@session"| p20
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p20
  p10 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p20
  p10 -->|"dim_agent_runtime::scope::SessionLoader@session"| p20
  p25 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/base@session"| p18
  p25 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/environment/context@session"| p18
  p25 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/environment/date@session"| p18
  p25 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/instructions/project@session"| p18
  p25 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/instructions/user@session"| p18
  p25 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/session@session"| p18
  p20 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/skills/instructions@session"| p18
  p20 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/skills/list@session"| p18
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p18
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p17
  p14 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p17
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p17
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p17
  p10 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p17
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p16
  p14 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p16
  p5 -->|"dim_agent_capabilities::workspace_tools::files::FileWriteCoordinator"| p16
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p16
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p16
  p16 -->|"dim_agent_capabilities::host::ToolHost@session"| p15
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p15
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p15
  p10 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p15
  p10 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p14
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p14
  p10 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p14
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p13
  p15 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p12
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p12
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p11
```
