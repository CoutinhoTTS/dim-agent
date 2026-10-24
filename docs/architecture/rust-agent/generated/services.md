# 默认运行组合

由真实 rutis 装配生成。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p12["branch"]
    p24["ModelProvider"]
    p23["ToolExecutor"]
    p22["ToolBinding"]
    p21["Approval"]
    p20["Permission"]
    p19["Loop"]
    p18["InputQueue"]
    p17["Prompt"]
    p16["ContextPreparation"]
    p15["TreeTimeline"]
    p14["SessionBroadcast"]
    p13["BranchScope"]
  end
  subgraph session["session"]
    p5["session"]
    p11["InputMethods"]
    p10["TimelineTree"]
    p9["SessionBroadcastEnd"]
    p8["PrepareModelHost"]
    p7["ToolRegistry"]
    p6["SessionScope"]
  end
  p16 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p24
  p6 -->|"dim_agent_capabilities::host::ModelHost@session"| p24
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p24
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p24
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p24
  p21 -->|"dim_agent_capabilities::approval::Approval@main"| p23
  p22 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p23
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p23
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p23
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p23
  p17 -->|"dim_agent_capabilities::prompt::Prompt@main"| p22
  p7 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p22
  p13 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p22
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p22
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p22
  p13 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p22
  p20 -->|"dim_agent_capabilities::permission::Permission@main"| p21
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p21
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p21
  p6 -->|"dim_agent_capabilities::permission::CommandRulesHost@session"| p20
  p6 -->|"dim_agent_runtime::config::SessionConfig@session"| p20
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p20
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p20
  p6 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p19
  p6 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p19
  p6 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p19
  p8 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p19
  p22 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p19
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p19
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p19
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p18
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p18
  p16 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p17
  p6 -->|"dim_agent_capabilities::host::PromptHost@session"| p17
  p6 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p17
  p13 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p17
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p17
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p17
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p17
  p6 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p17
  p13 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p17
  p6 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p16
  p6 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p16
  p6 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p16
  p6 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p16
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p16
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p16
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p16
  p10 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p15
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p15
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p15
  p13 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p15
  p13 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p14
  p13 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p14
  p10 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p11
  p6 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p10
  p6 -->|"dim_agent_runtime::config::SessionIdentity@session"| p10
  p6 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p10
  p6 -->|"dim_agent_runtime::config::SessionIdentity@session"| p9
  p7 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p8
  p6 -->|"dim_agent_runtime::config::SessionConfig@session"| p8
  p6 -->|"dim_agent_capabilities::host::ToolHost@session"| p7
  p6 -->|"dim_agent_runtime::config::SessionConfig@session"| p7
  p6 -->|"dim_agent_runtime::config::SessionIdentity@session"| p7
  p6 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p7
```
