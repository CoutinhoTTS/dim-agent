# App Server 进程组合

由 `serve` 以产品配置（`--product-home`）真实装配生成，只含进程层，不含 Agent 实例。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph process["process"]
    p1["root"]
    p2["rutis-loader"]
    p54["Attachments"]
    p53["Questions"]
    p52["PrepareModelHost"]
    p51["ContextCompaction"]
    p50["CommandRules"]
    p49["PromptSources"]
    p48["ShellInput"]
    p47["Recall"]
    p46["Wait"]
    p45["WorkspaceTools"]
    p44["BranchTool"]
    p43["ToolExecution"]
    p42["Permission"]
    p41["ModelProvider"]
    p40["Loop"]
    p39["Prompt"]
    p38["ContextPreparation"]
    p37["ToolRegistry"]
    p36["InputQueue"]
    p35["SessionMethods"]
    p34["TimelineTree"]
    p33["SessionBroadcast"]
    p32["PartRegistry"]
    p31["Transport"]
    p30["Rpc"]
    p29["Connection"]
    p28["UiHost"]
    p27["Collaboration"]
    p26["Agents"]
    p25["CollaborationDirectory"]
    p24["AgentDirectory"]
    p23["PluginManager"]
    p22["CollaborationModelPolicy"]
    p21["PermissionRules"]
    p20["SkillsCatalog"]
    p19["IsolationPolicy"]
    p18["HostPlatform"]
    p17["Mcp"]
    p16["McpPublication"]
    p15["SettingsMethods"]
    p14["Settings"]
    p13["Provider"]
    p12["GrokOAuth"]
    p11["OpenAICodexOAuth"]
    p10["ManagedProviders"]
    p9["PluginFiles"]
    p8["ManagementMethods"]
    p7["SessionStore"]
    p6["SqliteStorage"]
    p5["Aimux"]
    p4["FileStorage"]
    p3["ServerControl"]
  end
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p54
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p53
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p52
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p51
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p50
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p49
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p48
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p47
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p46
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p45
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p44
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p43
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p42
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p41
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p40
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p39
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p38
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p37
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p36
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p35
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p34
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p33
  p29 -->|"dim_app_server::connection::ConnectionHub"| p31
  p3 -->|"dim_app_server::control::ServerControl"| p31
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p31
  p26 -->|"dim_app_server::agents::AgentIndex"| p30
  p27 -->|"dim_app_server::collaboration::Collaboration"| p30
  p29 -->|"dim_app_server::connection::ConnectionHub"| p30
  p3 -->|"dim_app_server::control::ServerControl"| p30
  p8 -->|"dim_app_server::methods::ManagementMethods"| p30
  p2 -->|"rutis_loader::loader::Loader"| p30
  p3 -->|"dim_app_server::control::ServerControl"| p29
  p28 -->|"dim_app_server::ui_host::UiHost"| p29
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p27
  p26 -->|"dim_app_server::agents::AgentIndex"| p27
  p25 -->|"dim_app_server::collaboration::CollaborationDirectory"| p27
  p14 -->|"dim_app_server::settings::SettingsService"| p27
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p26
  p24 -->|"dim_app_server::agents::AgentDirectory"| p26
  p14 -->|"dim_app_server::settings::SettingsService"| p26
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p26
  p7 -->|"dim_app_server::storage::timeline::SessionStore"| p26
  p2 -->|"rutis_loader::loader::Loader"| p26
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p23
  p8 -->|"dim_app_server::methods::ManagementMethods"| p23
  p2 -->|"rutis_loader::loader::Loader"| p23
  p8 -->|"dim_app_server::methods::ManagementMethods"| p22
  p14 -->|"dim_app_server::settings::SettingsService"| p22
  p8 -->|"dim_app_server::methods::ManagementMethods"| p21
  p14 -->|"dim_app_server::settings::SettingsService"| p21
  p18 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p20
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p20
  p8 -->|"dim_app_server::methods::ManagementMethods"| p20
  p14 -->|"dim_app_server::settings::SettingsService"| p20
  p18 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p19
  p8 -->|"dim_app_server::methods::ManagementMethods"| p19
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p18
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p17
  p32 -->|"dim_agent_runtime::parts::PartRegistry"| p17
  p16 -->|"dim_app_server::mcp::publication::Publication"| p17
  p8 -->|"dim_app_server::methods::ManagementMethods"| p17
  p14 -->|"dim_app_server::settings::SettingsService"| p17
  p14 -->|"dim_app_server::settings::SettingsService"| p16
  p8 -->|"dim_app_server::methods::ManagementMethods"| p15
  p14 -->|"dim_app_server::settings::SettingsService"| p15
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p14
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p14
  p5 -->|"dim_agent_capabilities::host::ModelHost"| p13
  p11 -->|"dim_app_server::codex_oauth::OpenAICodexOAuthService"| p13
  p12 -->|"dim_app_server::grok_oauth::GrokOAuthService"| p13
  p10 -->|"dim_app_server::managed_provider::ManagedProviders"| p13
  p8 -->|"dim_app_server::methods::ManagementMethods"| p13
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p13
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p12
  p8 -->|"dim_app_server::methods::ManagementMethods"| p12
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p11
  p8 -->|"dim_app_server::methods::ManagementMethods"| p11
  p8 -->|"dim_app_server::methods::ManagementMethods"| p10
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p9
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p7
```
