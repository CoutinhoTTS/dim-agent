# 本地执行运行组合

由真实 rutis 装配 `defaults::install_local_execution` 生成（模型为脚本化 mock）。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p19["branch"]
    p36["SkillsBranch"]
    p35["PromptSourcesBranch"]
    p34["WaitBranch"]
    p33["BackgroundExec"]
    p32["FileReads"]
    p31["ToolExecutor"]
    p30["ToolBinding"]
    p29["Approval"]
    p28["Permission"]
    p27["ModelProvider"]
    p26["Loop"]
    p25["InputQueue"]
    p24["Prompt"]
    p23["ContextPreparation"]
    p22["TreeTimeline"]
    p21["SessionBroadcast"]
    p20["BranchScope"]
  end
  subgraph process["process"]
    p5["rutis-loader"]
    p4["SkillsCatalog"]
    p3["HostPlatform"]
    p2["FileStorage"]
  end
  subgraph session["session"]
    p8["session"]
    p18["Skills"]
    p17["PromptSources"]
    p16["PromptRegistry"]
    p15["Wait"]
    p14["WorkspaceTools"]
    p13["ToolRegistry"]
    p12["TimelineTree"]
    p11["SessionBroadcastEnd"]
    p10["PrepareModelHost"]
    p9["SessionScope"]
  end
  p18 -->|"dim_agent_capabilities::skills::session::SessionSkills@session"| p36
  p20 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p36
  p24 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p36
  p16 -->|"dim_agent_capabilities::host::PromptHost@session"| p35
  p20 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p35
  p24 -->|"dim_agent_runtime::state::StateCell<dim_agent_capabilities::prompt_state::PromptSnapshot>@main"| p35
  p24 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p35
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p34
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p34
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p33
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p33
  p29 -->|"dim_agent_capabilities::approval::Approval@main"| p31
  p30 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p31
  p20 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p31
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p31
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p31
  p20 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p31
  p24 -->|"dim_agent_capabilities::prompt::Prompt@main"| p30
  p13 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p30
  p20 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p30
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p30
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p30
  p28 -->|"dim_agent_capabilities::permission::Permission@main"| p29
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p29
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p29
  p9 -->|"dim_agent_runtime::config::SessionConfig@session"| p28
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p28
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p28
  p23 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p27
  p9 -->|"dim_agent_capabilities::host::ModelHost@session"| p27
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p27
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p27
  p20 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p27
  p9 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p26
  p9 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p26
  p10 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p26
  p30 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p26
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p26
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p26
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p25
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p25
  p23 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p24
  p16 -->|"dim_agent_capabilities::host::PromptHost@session"| p24
  p16 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p24
  p20 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p24
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p24
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p24
  p20 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p24
  p9 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p24
  p9 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p23
  p9 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p23
  p9 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p23
  p20 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p23
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p23
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p23
  p20 -->|"dim_agent_runtime::diagnostics::Diagnostics@main"| p23
  p12 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p22
  p20 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p22
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p22
  p20 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p22
  p20 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p21
  p20 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p21
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p4
  p2 -->|"dim_agent_capabilities::file_storage::FileStorage"| p3
  p4 -->|"dim_agent_capabilities::skills::catalog::SkillsCatalog"| p18
  p9 -->|"dim_agent_runtime::config::SessionConfig@session"| p18
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p17
  p9 -->|"dim_agent_runtime::config::SessionConfig@session"| p17
  p17 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/base@session"| p16
  p17 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/environment/context@session"| p16
  p17 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/environment/date@session"| p16
  p17 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/instructions/project@session"| p16
  p17 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/instructions/user@session"| p16
  p17 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/session@session"| p16
  p18 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/skills/instructions@session"| p16
  p18 -->|"dim_agent_capabilities::prompt_registry::PromptSource#dim/skills/list@session"| p16
  p9 -->|"dim_agent_runtime::config::SessionConfig@session"| p16
  p12 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p15
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p14
  p12 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p14
  p9 -->|"dim_agent_runtime::config::SessionConfig@session"| p14
  p9 -->|"dim_agent_runtime::config::SessionIdentity@session"| p14
  p14 -->|"dim_agent_capabilities::host::ToolHost@session"| p13
  p12 -->|"dim_agent_capabilities::tool_registry::ToolContribution#branch@session"| p13
  p18 -->|"dim_agent_capabilities::tool_registry::ToolContribution#skill@session"| p13
  p15 -->|"dim_agent_capabilities::tool_registry::ToolContribution#wait@session"| p13
  p9 -->|"dim_agent_runtime::config::SessionConfig@session"| p13
  p9 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p12
  p9 -->|"dim_agent_runtime::config::SessionIdentity@session"| p12
  p9 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p12
  p9 -->|"dim_agent_runtime::config::SessionIdentity@session"| p11
  p13 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p10
  p9 -->|"dim_agent_runtime::config::SessionConfig@session"| p10
```
