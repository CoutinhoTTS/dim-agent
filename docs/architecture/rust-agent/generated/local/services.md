# 本地执行运行组合

由真实 rutis 装配 `defaults::install_local_execution` 生成（模型为脚本化 mock）。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p20["branch"]
    p37["SkillsBranch"]
    p36["PromptSourcesBranch"]
    p35["WaitBranch"]
    p34["BackgroundExec"]
    p33["FileReads"]
    p32["ToolExecutor"]
    p31["ToolBinding"]
    p30["Approval"]
    p29["Permission"]
    p28["ModelProvider"]
    p27["Loop"]
    p26["InputQueue"]
    p25["Prompt"]
    p24["ContextPreparation"]
    p23["TreeTimeline"]
    p22["SessionBroadcast"]
    p21["BranchScope"]
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
    p19["Skills"]
    p18["PromptSources"]
    p17["PromptRegistry"]
    p16["Wait"]
    p15["WorkspaceTools"]
    p14["ToolRegistry"]
    p13["TimelineTree"]
    p12["SessionBroadcastEnd"]
    p11["PrepareModelHost"]
    p10["SessionScope"]
  end
  p19 -->|"dim_agent_capabilities::skills::session::SessionSkills@session"| p37
  p21 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p37
  p25 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p37
  p17 -->|"dim_agent_capabilities::host::PromptHost@session"| p36
  p21 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p36
  p25 -->|"dim_agent_runtime::state::StateCell<dim_agent_capabilities::prompt_state::PromptSnapshot>@main"| p36
  p25 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p36
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p35
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p35
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p34
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p34
  p30 -->|"dim_agent_capabilities::approval::Approval@main"| p32
  p31 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p32
  p21 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p32
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p32
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p32
  p21 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p32
  p25 -->|"dim_agent_capabilities::prompt::Prompt@main"| p31
  p14 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p31
  p21 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p31
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p31
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p31
  p21 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p31
  p29 -->|"dim_agent_capabilities::permission::Permission@main"| p30
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p30
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p30
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p29
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p29
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p29
  p24 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p28
  p10 -->|"dim_agent_capabilities::host::ModelHost@session"| p28
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p28
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p28
  p21 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p28
  p10 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p27
  p10 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p27
  p11 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p27
  p31 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p27
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p27
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p27
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p26
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p26
  p24 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p25
  p17 -->|"dim_agent_capabilities::host::PromptHost@session"| p25
  p17 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p25
  p21 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p25
  p21 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p25
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p25
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p25
  p21 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p25
  p10 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p25
  p21 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p25
  p10 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p24
  p10 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p24
  p10 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p24
  p21 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p24
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p24
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p24
  p21 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p24
  p13 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p23
  p21 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p23
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p23
  p21 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p23
  p21 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p22
  p21 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p22
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p4
  p2 -->|"dim_agent_capabilities::file_storage::FileStorage"| p3
  p4 -->|"dim_agent_capabilities::skills::catalog::SkillsCatalog"| p19
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p19
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p18
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p18
  p18 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/base@session"| p17
  p18 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/environment/context@session"| p17
  p18 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/environment/date@session"| p17
  p18 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/instructions/project@session"| p17
  p18 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/instructions/user@session"| p17
  p18 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/session@session"| p17
  p19 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/skills/instructions@session"| p17
  p19 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/skills/list@session"| p17
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p17
  p13 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p16
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p15
  p13 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p15
  p5 -->|"dim_agent_capabilities::workspace_tools::files::FileWriteCoordinator"| p15
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p15
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p15
  p15 -->|"dim_agent_capabilities::host::ToolHost@session"| p14
  p13 -->|"dim_agent_capabilities::tool_registry::ToolContribution#branch@session"| p14
  p19 -->|"dim_agent_capabilities::tool_registry::ToolContribution#skill@session"| p14
  p16 -->|"dim_agent_capabilities::tool_registry::ToolContribution#wait@session"| p14
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p14
  p10 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p13
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p13
  p10 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p13
  p10 -->|"dim_agent_runtime::config::SessionIdentity@session"| p12
  p14 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p11
  p10 -->|"dim_agent_runtime::config::SessionConfig@session"| p11
```
