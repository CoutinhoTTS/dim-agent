# App Server 进程组合

由 `serve` 以产品配置（`--product-home`）真实装配生成，只含进程层，不含 Agent 实例。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph process["process"]
    p1["root"]
    p2["rutis-loader"]
    p51["Attachments"]
    p50["Questions"]
    p49["PrepareModelHost"]
    p48["ContextCompaction"]
    p47["CommandRules"]
    p46["PromptSources"]
    p45["ShellInput"]
    p44["Wait"]
    p43["WorkspaceTools"]
    p42["BranchTool"]
    p41["ToolExecution"]
    p40["Permission"]
    p39["ModelProvider"]
    p38["Loop"]
    p37["Prompt"]
    p36["ContextPreparation"]
    p35["ToolRegistry"]
    p34["InputQueue"]
    p33["SessionMethods"]
    p32["TimelineTree"]
    p31["SessionBroadcast"]
    p30["PartRegistry"]
    p29["Transport"]
    p28["Rpc"]
    p27["Connection"]
    p26["UiHost"]
    p25["Collaboration"]
    p24["Agents"]
    p23["CollaborationDirectory"]
    p22["AgentDirectory"]
    p21["CollaborationModelPolicy"]
    p20["PermissionRules"]
    p19["SkillsCatalog"]
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
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p51
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p50
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p49
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p48
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p47
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p46
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p45
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p44
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p43
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p42
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p41
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p40
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p39
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p38
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p37
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p36
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p35
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p34
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p33
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p32
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p31
  p27 -->|"dim_app_server::connection::ConnectionHub"| p29
  p3 -->|"dim_app_server::control::ServerControl"| p29
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p29
  p24 -->|"dim_app_server::agents::AgentIndex"| p28
  p25 -->|"dim_app_server::collaboration::Collaboration"| p28
  p27 -->|"dim_app_server::connection::ConnectionHub"| p28
  p3 -->|"dim_app_server::control::ServerControl"| p28
  p8 -->|"dim_app_server::methods::ManagementMethods"| p28
  p2 -->|"rutis_loader::loader::Loader"| p28
  p3 -->|"dim_app_server::control::ServerControl"| p27
  p26 -->|"dim_app_server::ui_host::UiHost"| p27
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p25
  p24 -->|"dim_app_server::agents::AgentIndex"| p25
  p23 -->|"dim_app_server::collaboration::CollaborationDirectory"| p25
  p14 -->|"dim_app_server::settings::SettingsService"| p25
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p24
  p22 -->|"dim_app_server::agents::AgentDirectory"| p24
  p14 -->|"dim_app_server::settings::SettingsService"| p24
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p24
  p7 -->|"dim_app_server::storage::timeline::SessionStore"| p24
  p2 -->|"rutis_loader::loader::Loader"| p24
  p8 -->|"dim_app_server::methods::ManagementMethods"| p21
  p14 -->|"dim_app_server::settings::SettingsService"| p21
  p8 -->|"dim_app_server::methods::ManagementMethods"| p20
  p14 -->|"dim_app_server::settings::SettingsService"| p20
  p18 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p19
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p19
  p8 -->|"dim_app_server::methods::ManagementMethods"| p19
  p14 -->|"dim_app_server::settings::SettingsService"| p19
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p18
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p17
  p30 -->|"dim_agent_runtime::parts::PartRegistry"| p17
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
