# 默认运行组合

由真实 rutis 装配生成。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p17["branch"]
    p20["BranchOptionalParts"]
    p19["BranchParts"]
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
    p18["BranchScope"]
  end
  subgraph session["session"]
    p6["session"]
    p9["SessionOptionalParts"]
    p8["SessionParts"]
    p16["BranchTool"]
    p15["ToolRegistry"]
    p14["InputMethods"]
    p13["SessionMethods"]
    p12["TimelineTree"]
    p11["SessionBroadcastEnd"]
    p10["PrepareModelHost"]
    p7["SessionScope"]
  end
  p29 -->|"dim_agent_capabilities::approval::Approval@main"| p31
  p30 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p31
  p18 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p31
  p18 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p31
  p18 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p31
  p24 -->|"dim_agent_capabilities::prompt::Prompt@main"| p30
  p15 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p30
  p18 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p30
  p18 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p30
  p18 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p30
  p18 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p30
  p28 -->|"dim_agent_capabilities::permission::Permission@main"| p29
  p18 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p29
  p18 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p29
  p7 -->|"dim_agent_capabilities::permission::CommandRulesHost@session"| p28
  p7 -->|"dim_agent_runtime::config::SessionConfig@session"| p28
  p18 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p28
  p18 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p28
  p23 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p27
  p18 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p27
  p18 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p27
  p18 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p27
  p7 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p26
  p7 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p26
  p7 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p26
  p10 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p26
  p30 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p26
  p18 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p26
  p18 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p26
  p18 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p25
  p18 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p25
  p23 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p24
  p7 -->|"dim_agent_capabilities::host::PromptHost@session"| p24
  p7 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p24
  p18 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p24
  p18 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p24
  p18 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p24
  p18 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p24
  p7 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p24
  p18 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p24
  p7 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p23
  p7 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p23
  p7 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p23
  p7 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p23
  p18 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p23
  p18 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p23
  p18 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p23
  p12 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p22
  p18 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p22
  p18 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p22
  p18 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p22
  p18 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p21
  p18 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p21
  p12 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p16
  p15 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p16
  p7 -->|"dim_agent_capabilities::host::ToolHost@session"| p15
  p7 -->|"dim_agent_runtime::config::SessionConfig@session"| p15
  p7 -->|"dim_agent_runtime::config::SessionIdentity@session"| p15
  p7 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p15
  p13 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p14
  p12 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p14
  p7 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p12
  p7 -->|"dim_agent_runtime::config::SessionIdentity@session"| p12
  p7 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p12
  p7 -->|"dim_agent_runtime::config::SessionIdentity@session"| p11
  p15 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p10
  p7 -->|"dim_agent_runtime::config::SessionConfig@session"| p10
```
