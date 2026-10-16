# App Server 进程组合

由 `serve` 以产品配置（`--product-home`）真实装配生成，只含进程层，不含 Agent 实例。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph process["process"]
    p1["root"]
    p2["rutis-loader"]
    p25["Transport"]
    p24["Rpc"]
    p23["Connection"]
    p22["Collaboration"]
    p21["Agents"]
    p20["CollaborationDirectory"]
    p19["AgentDirectory"]
    p18["PermissionRules"]
    p17["SkillsCatalog"]
    p16["HostPlatform"]
    p15["Mcp"]
    p14["McpPublication"]
    p13["Settings"]
    p12["Provider"]
    p11["GrokOAuth"]
    p10["OpenAICodexOAuth"]
    p9["ManagedProviders"]
    p8["SessionStore"]
    p7["SqliteStorage"]
    p6["Aimux"]
    p5["FileWriteCoordinator"]
    p4["FileStorage"]
    p3["ServerControl"]
  end
  p23 -->|"dim_app_server::connection::ConnectionHub"| p25
  p3 -->|"dim_app_server::control::ServerControl"| p25
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p25
  p21 -->|"dim_app_server::agents::AgentIndex"| p24
  p22 -->|"dim_app_server::collaboration::Collaboration"| p24
  p23 -->|"dim_app_server::connection::ConnectionHub"| p24
  p3 -->|"dim_app_server::control::ServerControl"| p24
  p2 -->|"rutis_loader::loader::Loader"| p24
  p3 -->|"dim_app_server::control::ServerControl"| p23
  p21 -->|"dim_app_server::agents::AgentIndex"| p22
  p20 -->|"dim_app_server::collaboration::CollaborationDirectory"| p22
  p19 -->|"dim_app_server::agents::AgentDirectory"| p21
  p13 -->|"dim_app_server::settings::SettingsService"| p21
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p21
  p8 -->|"dim_app_server::storage::timeline::SessionStore"| p21
  p2 -->|"rutis_loader::loader::Loader"| p21
  p13 -->|"dim_app_server::settings::SettingsService"| p18
  p16 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p17
  p13 -->|"dim_app_server::settings::SettingsService"| p17
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p16
  p14 -->|"dim_app_server::mcp::publication::Publication"| p15
  p13 -->|"dim_app_server::settings::SettingsService"| p15
  p13 -->|"dim_app_server::settings::SettingsService"| p14
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p13
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p13
  p6 -->|"dim_agent_capabilities::host::ModelHost"| p12
  p10 -->|"dim_app_server::codex_oauth::OpenAICodexOAuthService"| p12
  p11 -->|"dim_app_server::grok_oauth::GrokOAuthService"| p12
  p9 -->|"dim_app_server::managed_provider::ManagedProviders"| p12
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p12
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p11
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p10
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p8
```
