# 默认运行组合

由真实 rutis 装配生成。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p31["branch"]
    p34["BranchOptionalParts"]
    p33["BranchParts"]
    p45["Prompt"]
    p44["ToolExecutor"]
    p43["ToolBinding"]
    p42["ContextPreparation"]
    p41["Approval"]
    p40["Permission"]
    p39["ModelProvider"]
    p38["InputQueue"]
    p37["Loop"]
    p36["TreeTimeline"]
    p35["SessionBroadcast"]
    p32["BranchScope"]
  end
  subgraph session["session"]
    p20["session"]
    p23["SessionOptionalParts"]
    p22["SessionParts"]
    p30["BranchTool"]
    p29["ToolRegistry"]
    p28["InputMethods"]
    p27["SessionMethods"]
    p26["TimelineTree"]
    p25["PrepareModelHost"]
    p24["SessionBroadcastEnd"]
    p21["SessionScope"]
  end
  p42 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p45
  p21 -->|"dim_agent_capabilities::host::PromptHost@session"| p45
  p21 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p45
  p32 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p45
  p32 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p45
  p32 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p45
  p32 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p45
  p21 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p45
  p32 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p45
  p41 -->|"dim_agent_capabilities::approval::Approval@main"| p44
  p43 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p44
  p32 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p44
  p32 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p44
  p32 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p44
  p45 -->|"dim_agent_capabilities::prompt::Prompt@main"| p43
  p29 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p43
  p32 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p43
  p32 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p43
  p32 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p43
  p32 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p43
  p21 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p42
  p21 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p42
  p21 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p42
  p21 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p42
  p32 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p42
  p32 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p42
  p32 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p42
  p40 -->|"dim_agent_capabilities::permission::Permission@main"| p41
  p32 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p41
  p32 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p41
  p21 -->|"dim_agent_capabilities::permission::CommandRulesHost@session"| p40
  p21 -->|"dim_agent_runtime::config::SessionConfig@session"| p40
  p32 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p40
  p32 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p40
  p42 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p39
  p32 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p39
  p32 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p39
  p32 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p39
  p32 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p38
  p32 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p38
  p21 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p37
  p21 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p37
  p21 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p37
  p25 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p37
  p43 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p37
  p32 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p37
  p32 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p37
  p26 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p36
  p32 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p36
  p32 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p36
  p32 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p36
  p32 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p35
  p32 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p35
  p26 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p30
  p29 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p30
  p21 -->|"dim_agent_capabilities::host::ToolHost@session"| p29
  p21 -->|"dim_agent_runtime::config::SessionConfig@session"| p29
  p21 -->|"dim_agent_runtime::config::SessionIdentity@session"| p29
  p21 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p29
  p27 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p28
  p26 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p28
  p21 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p26
  p21 -->|"dim_agent_runtime::config::SessionIdentity@session"| p26
  p21 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p26
  p29 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p25
  p21 -->|"dim_agent_runtime::config::SessionConfig@session"| p25
  p21 -->|"dim_agent_runtime::config::SessionIdentity@session"| p24
```
