# 本地执行运行组合

由真实 rutis 装配 `defaults::install_local_execution` 生成（模型为脚本化 mock）。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p53["branch"]
    p56["BranchOptionalParts"]
    p55["BranchParts"]
    p72["SkillsBranch"]
    p71["PromptSourcesBranch"]
    p70["WaitBranch"]
    p69["BackgroundExec"]
    p68["FileReads"]
    p67["ToolExecutor"]
    p66["ToolBinding"]
    p65["Approval"]
    p64["Permission"]
    p63["ModelProvider"]
    p62["Prompt"]
    p61["ContextPreparation"]
    p60["InputQueue"]
    p59["Loop"]
    p58["TreeTimeline"]
    p57["SessionBroadcast"]
    p54["BranchScope"]
  end
  subgraph process["process"]
    p5["rutis-loader"]
    p4["SkillsCatalog"]
    p3["HostPlatform"]
    p2["FileStorage"]
  end
  subgraph session["session"]
    p30["session"]
    p51["SkillContentInstances"]
    p52["SkillContent"]
    p34["SessionOptionalParts"]
    p33["SessionParts"]
    p50["Questions"]
    p49["Attachments"]
    p48["Skills"]
    p47["SkillContributions"]
    p46["PromptSources"]
    p45["PromptRegistry"]
    p44["ShellInput"]
    p43["Wait"]
    p42["WorkspaceTools"]
    p41["BranchTool"]
    p40["ToolRegistry"]
    p39["InputMethods"]
    p38["TimelineTree"]
    p37["SessionBroadcastEnd"]
    p36["SessionMethods"]
    p35["PrepareModelHost"]
    p32["SessionExtension"]
    p31["SessionScope"]
  end
  p48 -->|"dim_agent_capabilities::skills::session::SessionSkills@session"| p72
  p54 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p72
  p62 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p72
  p45 -->|"dim_agent_capabilities::host::PromptHost@session"| p71
  p54 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p71
  p62 -->|"dim_agent_runtime::state::StateCell<dim_agent_capabilities::prompt_state::PromptSnapshot>@main"| p71
  p62 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p71
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p70
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p70
  p31 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p69
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p69
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p69
  p65 -->|"dim_agent_capabilities::approval::Approval@main"| p67
  p66 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p67
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p67
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p67
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p67
  p62 -->|"dim_agent_capabilities::prompt::Prompt@main"| p66
  p40 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p66
  p54 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p66
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p66
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p66
  p54 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p66
  p64 -->|"dim_agent_capabilities::permission::Permission@main"| p65
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p65
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p65
  p31 -->|"dim_agent_capabilities::permission::CommandRulesHost@session"| p64
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p64
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p64
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p64
  p61 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p63
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p63
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p63
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p63
  p61 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p62
  p45 -->|"dim_agent_capabilities::host::PromptHost@session"| p62
  p45 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p62
  p54 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p62
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p62
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p62
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p62
  p31 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p62
  p54 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p62
  p31 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p61
  p31 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p61
  p31 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p61
  p31 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p61
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p61
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p61
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p61
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p60
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p60
  p31 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p59
  p31 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p59
  p31 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p59
  p35 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p59
  p66 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p59
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p59
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p59
  p38 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p58
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p58
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p58
  p54 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p58
  p54 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p57
  p54 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p57
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p4
  p2 -->|"dim_agent_capabilities::file_storage::FileStorage"| p3
  p47 -->|"dim_agent_capabilities::skills::content::SkillContributions@session"| p52
  p36 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p50
  p40 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p50
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p50
  p36 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p49
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p49
  p45 -->|"dim_agent_capabilities::prompt_registry::PromptRegistry@session"| p48
  p4 -->|"dim_agent_capabilities::skills::catalog::SkillsCatalog"| p48
  p47 -->|"dim_agent_capabilities::skills::content::SkillContributions@session"| p48
  p40 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p48
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p48
  p31 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p48
  p31 -->|"dim_agent_runtime::scope::SessionLoader@session"| p48
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p46
  p45 -->|"dim_agent_capabilities::prompt_registry::PromptRegistry@session"| p46
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p46
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p45
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p44
  p36 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p44
  p38 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p44
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p44
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p44
  p31 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p44
  p38 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p43
  p40 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p43
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p42
  p36 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p42
  p38 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p42
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p42
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p42
  p38 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p41
  p40 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p41
  p42 -->|"dim_agent_capabilities::host::ToolHost@session"| p40
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p40
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p40
  p31 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p40
  p36 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p39
  p38 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p39
  p31 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p38
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p38
  p31 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p38
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p37
  p40 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p35
  p31 -->|"dim_agent_runtime::config::SessionConfig@session"| p35
  p31 -->|"dim_agent_runtime::config::SessionIdentity@session"| p32
```
