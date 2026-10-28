# 本地执行运行组合

由真实 rutis 装配 `defaults::install_local_execution` 生成（模型为脚本化 mock）。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p53["branch"]
    p56["BranchOptionalParts"]
    p55["BranchParts"]
    p73["WaitBranch"]
    p72["TreeTimeline"]
    p71["ToolExecutor"]
    p70["ToolBinding"]
    p69["SkillsBranch"]
    p68["SessionBroadcast"]
    p67["PromptSourcesBranch"]
    p66["Prompt"]
    p65["Permission"]
    p64["ModelProvider"]
    p63["Loop"]
    p62["InputQueue"]
    p61["FileReads"]
    p60["ContextPreparation"]
    p59["BranchLine"]
    p58["BackgroundExec"]
    p57["Approval"]
    p54["BranchScope"]
  end
  subgraph process["process"]
    p4["rutis-loader"]
    p26["SkillsCatalog"]
    p3["HostPlatform"]
    p2["FileStorage"]
  end
  subgraph session["session"]
    p30["session"]
    p51["SkillContentInstances"]
    p52["SkillContent"]
    p34["SessionOptionalParts"]
    p33["SessionParts"]
    p50["WorkspaceTools"]
    p49["Wait"]
    p48["ToolRegistry"]
    p47["TimelineTree"]
    p46["Skills"]
    p45["ShellInput"]
    p44["SessionMethods"]
    p43["SessionBroadcastEnd"]
    p42["Recall"]
    p41["Questions"]
    p40["PromptSources"]
    p39["PromptRegistry"]
    p38["PrepareModelHost"]
    p37["InputMethods"]
    p36["BranchTool"]
    p35["Attachments"]
    p32["SessionExtension"]
    p31["SessionScope"]
  end
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p73
  p47 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p72
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p72
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p72
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p72
  p57 -->|"dim_agent_capabilities::approval::Approval@main"| p71
  p70 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p71
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p71
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p71
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p71
  p66 -->|"dim_agent_capabilities::prompt::Prompt@main"| p70
  p48 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p70
  p54 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p70
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p70
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p70
  p54 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p70
  p46 -->|"dim_agent_capabilities::skills::session::SessionSkills@session"| p69
  p54 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p69
  p66 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p69
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p68
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p68
  p39 -->|"dim_agent_capabilities::host::PromptHost@session"| p67
  p54 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p67
  p66 -->|"dim_agent_runtime::state::StateCell<dim_agent_capabilities::prompt_state::PromptSnapshot>@main"| p67
  p66 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p67
  p60 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p66
  p39 -->|"dim_agent_capabilities::host::PromptHost@session"| p66
  p39 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p66
  p54 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p66
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p66
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p66
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p66
  p31 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p66
  p54 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p66
  p31 -->|"dim_agent_capabilities::permission::CommandRulesHost@session"| p65
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p65
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p65
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p65
  p60 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p64
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p64
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p64
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p64
  p31 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p63
  p31 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p63
  p31 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p63
  p38 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p63
  p70 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p63
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p63
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p63
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p62
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p62
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p61
  p31 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p60
  p31 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p60
  p31 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p60
  p31 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p60
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p60
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p60
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p60
  p47 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p59
  p54 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p59
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p59
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p59
  p31 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p58
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p58
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p58
  p65 -->|"dim_agent_capabilities::permission::Permission@main"| p57
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p57
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p57
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p26
  p2 -->|"dim_agent_capabilities::file_storage::FileStorage"| p3
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p50
  p44 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p50
  p47 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p50
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p50
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p50
  p47 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p49
  p48 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p49
  p50 -->|"dim_agent_capabilities::host::ToolHost@session"| p48
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p48
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p48
  p31 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p48
  p31 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p47
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p47
  p31 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p47
  p39 -->|"dim_agent_capabilities::prompt_registry::PromptRegistry@session"| p46
  p26 -->|"dim_agent_capabilities::skills::catalog::SkillsCatalog"| p46
  p48 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p46
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p46
  p31 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p46
  p31 -->|"dim_agent_runtime::scope::SessionLoader@session"| p46
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p45
  p44 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p45
  p47 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p45
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p45
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p45
  p31 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p45
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p43
  p47 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p42
  p48 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p42
  p44 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p41
  p48 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p41
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p41
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p40
  p39 -->|"dim_agent_capabilities::prompt_registry::PromptRegistry@session"| p40
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p40
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p39
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p38
  p44 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p37
  p47 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p37
  p47 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p36
  p48 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p36
  p44 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p35
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p35
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p32
```
