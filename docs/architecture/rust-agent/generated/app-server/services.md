# App Server 进程组合

由 `serve` 以产品配置（`--product-home`）真实装配生成，只含进程层，不含 Agent 实例。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph process["process"]
    p1["root"]
    p2["rutis-loader"]
    p68["Attachments"]
    p67["goal"]
    p66["Questions"]
    p65["PromptSuggestion"]
    p64["Modality"]
    p63["PrepareModelHost"]
    p62["ContextCompaction"]
    p61["CommandRules"]
    p60["PromptSources"]
    p59["ShellInput"]
    p58["Recall"]
    p57["Wait"]
    p56["WorkspaceToolsOwner"]
    p55["BranchTool"]
    p54["ToolExecution"]
    p53["Permission"]
    p52["ModelProvider"]
    p51["Loop"]
    p50["Prompt"]
    p49["ContextPreparation"]
    p48["ToolRegistry"]
    p47["InputQueue"]
    p46["SessionMethods"]
    p45["TimelineTree"]
    p44["SessionBroadcast"]
    p43["session-title"]
    p42["PartRegistry"]
    p41["Transport"]
    p40["Rpc"]
    p39["Connection"]
    p38["UiHost"]
    p37["Collaboration"]
    p36["Agents"]
    p35["CollaborationDirectory"]
    p34["AgentDirectory"]
    p33["PluginManager"]
    p32["UiExtensions"]
    p31["CollaborationModelPolicy"]
    p30["HooksSettings"]
    p29["PermissionRules"]
    p28["SkillsCatalog"]
    p27["ExecutionBackend"]
    p26["ExecShellSettings"]
    p25["HostPlatform"]
    p24["PromptSuggestionSettings"]
    p23["ExecutionSettings"]
    p22["Mcp"]
    p21["McpPublication"]
    p20["ModelLog"]
    p19["web"]
    p18["SettingsMethods"]
    p17["Settings"]
    p16["Provider"]
    p15["GrokOAuth"]
    p14["OpenAICodexOAuth"]
    p13["ManagedProviderMethods"]
    p12["PluginFiles"]
    p11["ManagedProviders"]
    p10["CallChannel"]
    p9["WorkspaceFiles"]
    p8["ManagementMethods"]
    p7["SessionStore"]
    p6["SqliteStorage"]
    p5["Aimux"]
    p4["FileStorage"]
    p3["ServerControl"]
  end
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p68
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p67
  p17 -->|"dim_app_server::settings::SettingsService"| p67
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p66
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p65
  p16 -->|"dim_agent_capabilities::host::ModelSelectionHost"| p64
  p16 -->|"dim_agent_capabilities::host::SelectableModelsHost"| p64
  p8 -->|"dim_app_server::methods::ManagementMethods"| p64
  p12 -->|"dim_app_server::plugin_files::PluginFileTransactions"| p64
  p17 -->|"dim_app_server::settings::SettingsService"| p64
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p63
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p62
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p61
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p60
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p59
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p58
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p57
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p56
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p55
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p54
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p53
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p52
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p51
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p50
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p49
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p48
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p47
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p46
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p45
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p44
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p43
  p17 -->|"dim_app_server::settings::SettingsService"| p43
  p39 -->|"dim_app_server::connection::ConnectionHub"| p41
  p3 -->|"dim_app_server::control::ServerControl"| p41
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p41
  p10 -->|"dim_agent_capabilities::methods::Callers"| p40
  p36 -->|"dim_app_server::agents::AgentIndex"| p40
  p37 -->|"dim_app_server::collaboration::Collaboration"| p40
  p39 -->|"dim_app_server::connection::ConnectionHub"| p40
  p3 -->|"dim_app_server::control::ServerControl"| p40
  p8 -->|"dim_app_server::methods::ManagementMethods"| p40
  p2 -->|"rutis_loader::loader::Loader"| p40
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p39
  p3 -->|"dim_app_server::control::ServerControl"| p39
  p38 -->|"dim_app_server::ui_host::UiHost"| p39
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p37
  p36 -->|"dim_app_server::agents::AgentIndex"| p37
  p35 -->|"dim_app_server::collaboration::CollaborationDirectory"| p37
  p17 -->|"dim_app_server::settings::SettingsService"| p37
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p36
  p34 -->|"dim_app_server::agents::AgentDirectory"| p36
  p17 -->|"dim_app_server::settings::SettingsService"| p36
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p36
  p7 -->|"dim_app_server::storage::timeline::SessionStore"| p36
  p2 -->|"rutis_loader::loader::Loader"| p36
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p33
  p8 -->|"dim_app_server::methods::ManagementMethods"| p33
  p2 -->|"rutis_loader::loader::Loader"| p33
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p32
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p32
  p8 -->|"dim_app_server::methods::ManagementMethods"| p32
  p8 -->|"dim_app_server::methods::ManagementMethods"| p31
  p17 -->|"dim_app_server::settings::SettingsService"| p31
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p30
  p8 -->|"dim_app_server::methods::ManagementMethods"| p30
  p17 -->|"dim_app_server::settings::SettingsService"| p30
  p8 -->|"dim_app_server::methods::ManagementMethods"| p29
  p17 -->|"dim_app_server::settings::SettingsService"| p29
  p25 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p28
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p28
  p8 -->|"dim_app_server::methods::ManagementMethods"| p28
  p17 -->|"dim_app_server::settings::SettingsService"| p28
  p25 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p27
  p8 -->|"dim_app_server::methods::ManagementMethods"| p27
  p25 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p26
  p8 -->|"dim_app_server::methods::ManagementMethods"| p26
  p23 -->|"dim_agent_capabilities::execution_settings::ExecutionSettingsHost"| p25
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p25
  p17 -->|"dim_app_server::settings::SettingsService"| p24
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p23
  p17 -->|"dim_app_server::settings::SettingsService"| p23
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p22
  p1 -->|"dim_agent_capabilities::host::CredentialVariablesHost"| p22
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p22
  p21 -->|"dim_app_server::mcp::publication::Publication"| p22
  p8 -->|"dim_app_server::methods::ManagementMethods"| p22
  p17 -->|"dim_app_server::settings::SettingsService"| p22
  p1 -->|"dim_agent_capabilities::host::CredentialVariablesHost"| p21
  p17 -->|"dim_app_server::settings::SettingsService"| p21
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p20
  p17 -->|"dim_app_server::settings::SettingsService"| p20
  p42 -->|"dim_agent_runtime::parts::PartRegistry"| p19
  p17 -->|"dim_app_server::settings::SettingsService"| p19
  p8 -->|"dim_app_server::methods::ManagementMethods"| p18
  p17 -->|"dim_app_server::settings::SettingsService"| p18
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p17
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p17
  p5 -->|"dim_agent_capabilities::host::ModelHost"| p16
  p36 -->|"dim_app_server::agents::AgentIndex"| p16
  p14 -->|"dim_app_server::codex_oauth::OpenAICodexOAuthService"| p16
  p15 -->|"dim_app_server::grok_oauth::GrokOAuthService"| p16
  p11 -->|"dim_app_server::managed_provider::ManagedProviders"| p16
  p8 -->|"dim_app_server::methods::ManagementMethods"| p16
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p16
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p15
  p8 -->|"dim_app_server::methods::ManagementMethods"| p15
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p14
  p8 -->|"dim_app_server::methods::ManagementMethods"| p14
  p11 -->|"dim_app_server::managed_provider::ManagedProviders"| p13
  p8 -->|"dim_app_server::methods::ManagementMethods"| p13
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p12
  p56 -->|"dim_agent_capabilities::workspace_tools::mentions::MentionIndex"| p9
  p8 -->|"dim_app_server::methods::ManagementMethods"| p9
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p7
```
