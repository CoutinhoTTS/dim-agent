# App Server 进程组合

由 `serve` 以产品配置（`--product-home`）真实装配生成，只含进程层，不含 Agent 实例。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph process["process"]
    p1["root"]
    p2["rutis-loader"]
    p55["Attachments"]
    p54["Questions"]
    p53["PrepareModelHost"]
    p52["ContextCompaction"]
    p51["CommandRules"]
    p50["PromptSources"]
    p49["ShellInput"]
    p48["Recall"]
    p47["Wait"]
    p46["WorkspaceTools"]
    p45["BranchTool"]
    p44["ToolExecution"]
    p43["Permission"]
    p42["ModelProvider"]
    p41["Loop"]
    p40["Prompt"]
    p39["ContextPreparation"]
    p38["ToolRegistry"]
    p37["InputQueue"]
    p36["SessionMethods"]
    p35["TimelineTree"]
    p34["SessionBroadcast"]
    p33["PartRegistry"]
    p32["Transport"]
    p31["Rpc"]
    p30["Connection"]
    p29["UiHost"]
    p28["Collaboration"]
    p27["Agents"]
    p26["CollaborationDirectory"]
    p25["AgentDirectory"]
    p24["PluginManager"]
    p23["CollaborationModelPolicy"]
    p22["PermissionRules"]
    p21["SkillsCatalog"]
    p20["IsolationPolicy"]
    p19["HostPlatform"]
    p18["Mcp"]
    p17["McpPublication"]
    p16["web"]
    p15["SettingsMethods"]
    p14["Settings"]
    p13["Provider"]
    p12["GrokOAuth"]
    p11["OpenAICodexOAuth"]
    p10["PluginFiles"]
    p9["ManagedProviders"]
    p8["ManagementMethods"]
    p7["SessionStore"]
    p6["SqliteStorage"]
    p5["Aimux"]
    p4["FileStorage"]
    p3["ServerControl"]
  end
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p55
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p54
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p53
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p52
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p51
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p50
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p49
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p48
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p47
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p46
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p45
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p44
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p43
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p42
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p41
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p40
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p39
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p38
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p37
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p36
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p35
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p34
  p30 -->|"dim_app_server::connection::ConnectionHub"| p32
  p3 -->|"dim_app_server::control::ServerControl"| p32
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p32
  p27 -->|"dim_app_server::agents::AgentIndex"| p31
  p28 -->|"dim_app_server::collaboration::Collaboration"| p31
  p30 -->|"dim_app_server::connection::ConnectionHub"| p31
  p3 -->|"dim_app_server::control::ServerControl"| p31
  p8 -->|"dim_app_server::methods::ManagementMethods"| p31
  p2 -->|"rutis_loader::loader::Loader"| p31
  p3 -->|"dim_app_server::control::ServerControl"| p30
  p29 -->|"dim_app_server::ui_host::UiHost"| p30
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p28
  p27 -->|"dim_app_server::agents::AgentIndex"| p28
  p26 -->|"dim_app_server::collaboration::CollaborationDirectory"| p28
  p14 -->|"dim_app_server::settings::SettingsService"| p28
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p27
  p25 -->|"dim_app_server::agents::AgentDirectory"| p27
  p14 -->|"dim_app_server::settings::SettingsService"| p27
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p27
  p7 -->|"dim_app_server::storage::timeline::SessionStore"| p27
  p2 -->|"rutis_loader::loader::Loader"| p27
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p24
  p8 -->|"dim_app_server::methods::ManagementMethods"| p24
  p2 -->|"rutis_loader::loader::Loader"| p24
  p8 -->|"dim_app_server::methods::ManagementMethods"| p23
  p14 -->|"dim_app_server::settings::SettingsService"| p23
  p8 -->|"dim_app_server::methods::ManagementMethods"| p22
  p14 -->|"dim_app_server::settings::SettingsService"| p22
  p19 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p21
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p21
  p8 -->|"dim_app_server::methods::ManagementMethods"| p21
  p14 -->|"dim_app_server::settings::SettingsService"| p21
  p19 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p20
  p8 -->|"dim_app_server::methods::ManagementMethods"| p20
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p19
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p18
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p18
  p17 -->|"dim_app_server::mcp::publication::Publication"| p18
  p8 -->|"dim_app_server::methods::ManagementMethods"| p18
  p14 -->|"dim_app_server::settings::SettingsService"| p18
  p14 -->|"dim_app_server::settings::SettingsService"| p17
  p33 -->|"dim_agent_runtime::parts::PartRegistry"| p16
  p14 -->|"dim_app_server::settings::SettingsService"| p16
  p8 -->|"dim_app_server::methods::ManagementMethods"| p15
  p14 -->|"dim_app_server::settings::SettingsService"| p15
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p14
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p14
  p5 -->|"dim_agent_capabilities::host::ModelHost"| p13
  p11 -->|"dim_app_server::codex_oauth::OpenAICodexOAuthService"| p13
  p12 -->|"dim_app_server::grok_oauth::GrokOAuthService"| p13
  p9 -->|"dim_app_server::managed_provider::ManagedProviders"| p13
  p8 -->|"dim_app_server::methods::ManagementMethods"| p13
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p13
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p12
  p8 -->|"dim_app_server::methods::ManagementMethods"| p12
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p11
  p8 -->|"dim_app_server::methods::ManagementMethods"| p11
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p10
  p8 -->|"dim_app_server::methods::ManagementMethods"| p9
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p7
```
