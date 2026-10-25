# 本地执行运行组合

由真实 rutis 装配 `defaults::install_local_execution` 生成（模型为脚本化 mock）。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p52["branch"]
    p55["BranchOptionalParts"]
    p54["BranchParts"]
    p71["WaitBranch"]
    p70["TreeTimeline"]
    p69["ToolExecutor"]
    p68["ToolBinding"]
    p67["SkillsBranch"]
    p66["SessionBroadcast"]
    p65["PromptSourcesBranch"]
    p64["Prompt"]
    p63["Permission"]
    p62["ModelProvider"]
    p61["Loop"]
    p60["InputQueue"]
    p59["FileReads"]
    p58["ContextPreparation"]
    p57["BackgroundExec"]
    p56["Approval"]
    p53["BranchScope"]
  end
  subgraph process["process"]
    p4["rutis-loader"]
    p25["SkillsCatalog"]
    p3["HostPlatform"]
    p2["FileStorage"]
  end
  subgraph session["session"]
    p29["session"]
    p50["SkillContentInstances"]
    p51["SkillContent"]
    p33["SessionOptionalParts"]
    p32["SessionParts"]
    p49["WorkspaceTools"]
    p48["Wait"]
    p47["ToolRegistry"]
    p46["TimelineTree"]
    p45["Skills"]
    p44["SkillContributions"]
    p43["ShellInput"]
    p42["SessionMethods"]
    p41["SessionBroadcastEnd"]
    p40["Questions"]
    p39["PromptSources"]
    p38["PromptRegistry"]
    p37["PrepareModelHost"]
    p36["InputMethods"]
    p35["BranchTool"]
    p34["Attachments"]
    p31["SessionExtension"]
    p30["SessionScope"]
  end
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p71
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p71
  p46 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p70
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p70
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p70
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p70
  p56 -->|"dim_agent_capabilities::approval::Approval@main"| p69
  p68 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p69
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p69
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p69
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p69
  p64 -->|"dim_agent_capabilities::prompt::Prompt@main"| p68
  p47 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p68
  p53 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p68
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p68
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p68
  p53 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p68
  p45 -->|"dim_agent_capabilities::skills::session::SessionSkills@session"| p67
  p53 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p67
  p64 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p67
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p66
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p66
  p38 -->|"dim_agent_capabilities::host::PromptHost@session"| p65
  p53 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p65
  p64 -->|"dim_agent_runtime::state::StateCell<dim_agent_capabilities::prompt_state::PromptSnapshot>@main"| p65
  p64 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p65
  p58 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p64
  p38 -->|"dim_agent_capabilities::host::PromptHost@session"| p64
  p38 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p64
  p53 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p64
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p64
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p64
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p64
  p30 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p64
  p53 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p64
  p30 -->|"dim_agent_capabilities::permission::CommandRulesHost@session"| p63
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p63
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p63
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p63
  p58 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p62
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p62
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p62
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p62
  p30 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p61
  p30 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p61
  p30 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p61
  p37 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p61
  p68 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p61
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p61
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p61
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p60
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p60
  p30 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p58
  p30 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p58
  p30 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p58
  p30 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p58
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p58
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p58
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p58
  p30 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p57
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p57
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p57
  p63 -->|"dim_agent_capabilities::permission::Permission@main"| p56
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p56
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p56
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p25
  p2 -->|"dim_agent_capabilities::file_storage::FileStorage"| p3
  p44 -->|"dim_agent_capabilities::skills::content::SkillContributions@session"| p51
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p49
  p42 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p49
  p46 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p49
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p49
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p49
  p46 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p48
  p47 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p48
  p49 -->|"dim_agent_capabilities::host::ToolHost@session"| p47
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p47
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p47
  p30 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p47
  p30 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p46
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p46
  p30 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p46
  p38 -->|"dim_agent_capabilities::prompt_registry::PromptRegistry@session"| p45
  p25 -->|"dim_agent_capabilities::skills::catalog::SkillsCatalog"| p45
  p44 -->|"dim_agent_capabilities::skills::content::SkillContributions@session"| p45
  p47 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p45
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p45
  p30 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p45
  p30 -->|"dim_agent_runtime::scope::SessionLoader@session"| p45
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p43
  p42 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p43
  p46 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p43
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p43
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p43
  p30 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p43
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p41
  p42 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p40
  p47 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p40
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p40
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p39
  p38 -->|"dim_agent_capabilities::prompt_registry::PromptRegistry@session"| p39
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p39
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p38
  p47 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p37
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p37
  p42 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p36
  p46 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p36
  p46 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p35
  p47 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p35
  p42 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p34
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p34
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p31
```
