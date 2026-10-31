# App Server 进程组合

由 `serve` 以产品配置（`--product-home`）真实装配生成，只含进程层，不含 Agent 实例。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph process["process"]
    p1["root"]
    p2["rutis-loader"]
    p59["Attachments"]
    p58["Questions"]
    p57["PrepareModelHost"]
    p56["ContextCompaction"]
    p55["CommandRules"]
    p54["PromptSources"]
    p53["ShellInput"]
    p52["Recall"]
    p51["Wait"]
    p50["WorkspaceTools"]
    p49["BranchTool"]
    p48["ToolExecution"]
    p47["Permission"]
    p46["ModelProvider"]
    p45["Loop"]
    p44["Prompt"]
    p43["ContextPreparation"]
    p42["ToolRegistry"]
    p41["InputQueue"]
    p40["SessionMethods"]
    p39["TimelineTree"]
    p38["SessionBroadcast"]
    p37["PartRegistry"]
    p36["Transport"]
    p35["Rpc"]
    p34["Connection"]
    p33["UiHost"]
    p32["Collaboration"]
    p31["Agents"]
    p30["CollaborationDirectory"]
    p29["AgentDirectory"]
    p28["PluginManager"]
    p27["CollaborationModelPolicy"]
    p26["HooksSettings"]
    p25["PermissionRules"]
    p24["SkillsCatalog"]
    p23["ExecShellSettings"]
    p22["HostPlatform"]
    p21["ExecutionSettings"]
    p20["Mcp"]
    p19["McpPublication"]
    p18["ModelLog"]
    p17["web"]
    p16["SettingsMethods"]
    p15["Settings"]
    p14["Provider"]
    p13["GrokOAuth"]
    p12["OpenAICodexOAuth"]
    p11["ManagedProviderMethods"]
    p10["PluginFiles"]
    p9["ManagedProviders"]
    p8["ManagementMethods"]
    p7["SessionStore"]
    p6["SqliteStorage"]
    p5["Aimux"]
    p4["FileStorage"]
    p3["ServerControl"]
  end
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p59
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p58
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p57
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p56
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p55
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p54
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p53
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p52
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p51
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p50
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p49
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p48
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p47
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p46
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p45
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p44
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p43
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p42
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p41
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p40
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p39
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p38
  p34 -->|"dim_app_server::connection::ConnectionHub"| p36
  p3 -->|"dim_app_server::control::ServerControl"| p36
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p36
  p31 -->|"dim_app_server::agents::AgentIndex"| p35
  p32 -->|"dim_app_server::collaboration::Collaboration"| p35
  p34 -->|"dim_app_server::connection::ConnectionHub"| p35
  p3 -->|"dim_app_server::control::ServerControl"| p35
  p8 -->|"dim_app_server::methods::ManagementMethods"| p35
  p2 -->|"rutis_loader::loader::Loader"| p35
  p3 -->|"dim_app_server::control::ServerControl"| p34
  p33 -->|"dim_app_server::ui_host::UiHost"| p34
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p32
  p31 -->|"dim_app_server::agents::AgentIndex"| p32
  p30 -->|"dim_app_server::collaboration::CollaborationDirectory"| p32
  p15 -->|"dim_app_server::settings::SettingsService"| p32
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p31
  p29 -->|"dim_app_server::agents::AgentDirectory"| p31
  p15 -->|"dim_app_server::settings::SettingsService"| p31
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p31
  p7 -->|"dim_app_server::storage::timeline::SessionStore"| p31
  p2 -->|"rutis_loader::loader::Loader"| p31
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p28
  p8 -->|"dim_app_server::methods::ManagementMethods"| p28
  p2 -->|"rutis_loader::loader::Loader"| p28
  p8 -->|"dim_app_server::methods::ManagementMethods"| p27
  p15 -->|"dim_app_server::settings::SettingsService"| p27
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p26
  p8 -->|"dim_app_server::methods::ManagementMethods"| p26
  p15 -->|"dim_app_server::settings::SettingsService"| p26
  p8 -->|"dim_app_server::methods::ManagementMethods"| p25
  p15 -->|"dim_app_server::settings::SettingsService"| p25
  p22 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p24
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p24
  p8 -->|"dim_app_server::methods::ManagementMethods"| p24
  p15 -->|"dim_app_server::settings::SettingsService"| p24
  p22 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p23
  p8 -->|"dim_app_server::methods::ManagementMethods"| p23
  p21 -->|"dim_agent_capabilities::execution_settings::ExecutionSettingsHost"| p22
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p22
  p15 -->|"dim_app_server::settings::SettingsService"| p21
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p20
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p20
  p19 -->|"dim_app_server::mcp::publication::Publication"| p20
  p8 -->|"dim_app_server::methods::ManagementMethods"| p20
  p15 -->|"dim_app_server::settings::SettingsService"| p20
  p15 -->|"dim_app_server::settings::SettingsService"| p19
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p18
  p15 -->|"dim_app_server::settings::SettingsService"| p18
  p37 -->|"dim_agent_runtime::parts::PartRegistry"| p17
  p15 -->|"dim_app_server::settings::SettingsService"| p17
  p8 -->|"dim_app_server::methods::ManagementMethods"| p16
  p15 -->|"dim_app_server::settings::SettingsService"| p16
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p15
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p15
  p5 -->|"dim_agent_capabilities::host::ModelHost"| p14
  p12 -->|"dim_app_server::codex_oauth::OpenAICodexOAuthService"| p14
  p13 -->|"dim_app_server::grok_oauth::GrokOAuthService"| p14
  p9 -->|"dim_app_server::managed_provider::ManagedProviders"| p14
  p8 -->|"dim_app_server::methods::ManagementMethods"| p14
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p14
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p13
  p8 -->|"dim_app_server::methods::ManagementMethods"| p13
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p12
  p8 -->|"dim_app_server::methods::ManagementMethods"| p12
  p9 -->|"dim_app_server::managed_provider::ManagedProviders"| p11
  p8 -->|"dim_app_server::methods::ManagementMethods"| p11
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p10
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p7
```
