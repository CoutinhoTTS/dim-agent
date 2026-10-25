# 本地执行运行组合

由真实 rutis 装配 `defaults::install_local_execution` 生成（模型为脚本化 mock）。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph main["main"]
    p52["branch"]
    p55["BranchOptionalParts"]
    p54["BranchParts"]
    p71["SkillsBranch"]
    p70["PromptSourcesBranch"]
    p69["ToolExecutor"]
    p68["ToolBinding"]
    p67["Approval"]
    p66["Permission"]
    p65["WaitBranch"]
    p64["ModelProvider"]
    p63["BackgroundExec"]
    p62["FileReads"]
    p61["Loop"]
    p60["Prompt"]
    p59["ContextPreparation"]
    p58["InputQueue"]
    p57["SessionBroadcast"]
    p56["TreeTimeline"]
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
    p49["Skills"]
    p48["SkillContributions"]
    p47["Questions"]
    p46["Attachments"]
    p45["PromptSources"]
    p44["PromptRegistry"]
    p43["ShellInput"]
    p42["Wait"]
    p41["WorkspaceTools"]
    p40["BranchTool"]
    p39["InputMethods"]
    p38["SessionMethods"]
    p37["ToolRegistry"]
    p36["SessionBroadcastEnd"]
    p35["TimelineTree"]
    p34["PrepareModelHost"]
    p31["SessionExtension"]
    p30["SessionScope"]
  end
  p49 -->|"dim_agent_capabilities::skills::session::SessionSkills@session"| p71
  p53 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p71
  p60 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p71
  p44 -->|"dim_agent_capabilities::host::PromptHost@session"| p70
  p53 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p70
  p60 -->|"dim_agent_runtime::state::StateCell<dim_agent_capabilities::prompt_state::PromptSnapshot>@main"| p70
  p60 -->|"dyn dim_agent_runtime::contracts::ConfigureControl@main"| p70
  p67 -->|"dim_agent_capabilities::approval::Approval@main"| p69
  p68 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p69
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p69
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p69
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p69
  p60 -->|"dim_agent_capabilities::prompt::Prompt@main"| p68
  p37 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p68
  p53 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p68
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p68
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p68
  p53 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p68
  p66 -->|"dim_agent_capabilities::permission::Permission@main"| p67
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p67
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p67
  p30 -->|"dim_agent_capabilities::permission::CommandRulesHost@session"| p66
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p66
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p66
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p66
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p65
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p65
  p59 -->|"dim_agent_capabilities::context_preparation::ContextPreparation@main"| p64
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p64
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p64
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p64
  p30 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p63
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p63
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p63
  p30 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p61
  p30 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p61
  p30 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p61
  p34 -->|"dim_agent_capabilities::host::PrepareModelHost@session"| p61
  p68 -->|"dim_agent_capabilities::tool_binding::ToolBinding@main"| p61
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p61
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p61
  p59 -->|"dim_agent_capabilities::context_preparation::ConfigurationPublisher@main"| p60
  p44 -->|"dim_agent_capabilities::host::PromptHost@session"| p60
  p44 -->|"dim_agent_capabilities::prompt::InitialPromptConfiguration@session"| p60
  p53 -->|"dim_agent_runtime::config::BranchLifecycle@main"| p60
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p60
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p60
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p60
  p30 -->|"dim_agent_runtime::lifecycle::FaultReporter@session"| p60
  p53 -->|"dim_agent_runtime::timeline_context::BranchOpening@main"| p60
  p30 -->|"dim_agent_capabilities::history::HistoryRenderer@session"| p59
  p30 -->|"dim_agent_capabilities::host::CompactionStrategyHost@session"| p59
  p30 -->|"dim_agent_capabilities::host::ContextModelHost@session"| p59
  p30 -->|"dim_agent_capabilities::host::PrepareContextHost@session"| p59
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p59
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p59
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p59
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p58
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p58
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p57
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p57
  p35 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p56
  p53 -->|"dim_agent_runtime::config::TimelineIdentity@main"| p56
  p53 -->|"dim_agent_runtime::core::dispatcher::Dispatcher@main"| p56
  p53 -->|"dim_agent_runtime::core::timeline::Timeline@main"| p56
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p25
  p2 -->|"dim_agent_capabilities::file_storage::FileStorage"| p3
  p48 -->|"dim_agent_capabilities::skills::content::SkillContributions@session"| p51
  p44 -->|"dim_agent_capabilities::prompt_registry::PromptRegistry@session"| p49
  p25 -->|"dim_agent_capabilities::skills::catalog::SkillsCatalog"| p49
  p48 -->|"dim_agent_capabilities::skills::content::SkillContributions@session"| p49
  p37 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p49
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p49
  p30 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p49
  p30 -->|"dim_agent_runtime::scope::SessionLoader@session"| p49
  p38 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p47
  p37 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p47
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p47
  p38 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p46
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p46
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p45
  p44 -->|"dim_agent_capabilities::prompt_registry::PromptRegistry@session"| p45
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p45
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p44
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p43
  p38 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p43
  p35 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p43
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p43
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p43
  p30 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p43
  p35 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p42
  p37 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p42
  p3 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p41
  p38 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p41
  p35 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p41
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p41
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p41
  p35 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p40
  p37 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p40
  p38 -->|"dim_agent_capabilities::methods::SessionMethods@session"| p39
  p35 -->|"dim_agent_capabilities::timeline_tree::TreeCommands@session"| p39
  p41 -->|"dim_agent_capabilities::host::ToolHost@session"| p37
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p37
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p37
  p30 -->|"dim_agent_runtime::config::SessionLifecycle@session"| p37
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p36
  p30 -->|"dim_agent_runtime::assembly::TimelineFactory@session"| p35
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p35
  p30 -->|"dyn dim_agent_runtime::timeline_store::TimelineStore@session"| p35
  p37 -->|"dim_agent_capabilities::tool_registry::ToolRegistry@session"| p34
  p30 -->|"dim_agent_runtime::config::SessionConfig@session"| p34
  p30 -->|"dim_agent_runtime::config::SessionIdentity@session"| p31
```
