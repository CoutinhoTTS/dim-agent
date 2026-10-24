# 默认运行组合

由真实 rutis 装配生成。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p12["branch"]
    p24["ToolExecutor"]
    p23["ToolBinding"]
    p22["ModelProvider"]
    p21["Loop"]
    p20["Prompt"]
    p19["ContextPreparation"]
    p18["SessionBroadcast"]
    p17["Approval"]
    p16["Permission"]
    p15["InputQueue"]
    p14["TreeTimeline"]
    p13["BranchScope"]
  end
  subgraph session["session"]
    p5["session"]
    p11["InputMethods"]
    p10["ToolRegistry"]
    p9["TimelineTree"]
    p8["SessionBroadcastEnd"]
    p7["PrepareModelHost"]
    p6["SessionScope"]
  end
  p17 -->|"dim_agent_capabilities::approval::Approval@main"| p24
  p23 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p24
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p24
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p24
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p24
  p20 -->|"dim_agent_capabilities::prompt::Prompt@main"| p23
  p10 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p23
  p13 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p23
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p23
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p23
  p13 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p23
  p19 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p22
  p6 -->|"dim_agent_capabilities::host::ModelHost@session"| p22
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p22
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p22
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p22
  p6 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p21
  p6 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p21
  p6 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p21
  p7 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p21
  p23 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p21
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p21
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p21
  p19 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p20
  p6 -->|"dim_agent_capabilities::host::PromptHost@session"| p20
  p6 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p20
  p13 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p20
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p20
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p20
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p20
  p6 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p20
  p13 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p20
  p6 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p19
  p6 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p19
  p6 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p19
  p6 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p19
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p19
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p19
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p19
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p18
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p18
  p16 -->|"dim_agent_capabilities::permission::Permission@main"| p17
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p17
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p17
  p6 -->|"dim_agent_capabilities::permission::CommandRulesHost@session"| p16
  p6 -->|"dim_agent_runtime::config::SessionConfig@session"| p16
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p16
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p16
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p15
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p15
  p9 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p14
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p14
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p14
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p14
  p9 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p11
  p6 -->|"dim_agent_capabilities::host::ToolHost@session"| p10
  p6 -->|"dim_agent_runtime::config::SessionConfig@session"| p10
  p6 -->|"dim_agent_runtime::config::SessionIdentity@session"| p10
  p6 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p10
  p6 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p9
  p6 -->|"dim_agent_runtime::config::SessionIdentity@session"| p9
  p6 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p9
  p6 -->|"dim_agent_runtime::config::SessionIdentity@session"| p8
  p10 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p7
  p6 -->|"dim_agent_runtime::config::SessionConfig@session"| p7
```
