# App Server 进程组合

由 `serve` 以产品配置（`--product-home`）真实装配生成，只含进程层，不含 Agent 实例。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph process["process"]
    p1["root"]
    p2["rutis-loader"]
    p23["Transport"]
    p22["Rpc"]
    p21["Connection"]
    p20["Collaboration"]
    p19["Agents"]
    p18["CollaborationDirectory"]
    p17["AgentDirectory"]
    p16["SkillsCatalog"]
    p15["HostPlatform"]
    p14["Mcp"]
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
  p21 -->|"dim_app_server::connection::ConnectionHub"| p23
  p3 -->|"dim_app_server::control::ServerControl"| p23
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p23
  p16 -->|"dim_agent_capabilities::skills::catalog::SkillsCatalog"| p22
  p19 -->|"dim_app_server::agents::AgentIndex"| p22
  p10 -->|"dim_app_server::codex_oauth::OpenAICodexOAuthService"| p22
  p20 -->|"dim_app_server::collaboration::Collaboration"| p22
  p21 -->|"dim_app_server::connection::ConnectionHub"| p22
  p3 -->|"dim_app_server::control::ServerControl"| p22
  p11 -->|"dim_app_server::grok_oauth::GrokOAuthService"| p22
  p9 -->|"dim_app_server::managed_provider::ManagedProviders"| p22
  p14 -->|"dim_app_server::mcp::McpService"| p22
  p12 -->|"dim_app_server::provider::ProviderService"| p22
  p13 -->|"dim_app_server::settings::SettingsService"| p22
  p3 -->|"dim_app_server::control::ServerControl"| p21
  p19 -->|"dim_app_server::agents::AgentIndex"| p20
  p18 -->|"dim_app_server::collaboration::CollaborationDirectory"| p20
  p12 -->|"dim_agent_capabilities::host::ModelSelectionHost"| p19
  p17 -->|"dim_app_server::agents::AgentDirectory"| p19
  p13 -->|"dim_app_server::settings::SettingsService"| p19
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p19
  p8 -->|"dim_app_server::storage::timeline::SessionStore"| p19
  p2 -->|"rutis_loader::loader::Loader"| p19
  p15 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p16
  p13 -->|"dim_agent_capabilities::skill_settings::SkillSettingsHost"| p16
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p15
  p13 -->|"dim_app_server::settings::SettingsService"| p14
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p13
  p10 -->|"dim_app_server::codex_oauth::OpenAICodexOAuthService"| p12
  p11 -->|"dim_app_server::grok_oauth::GrokOAuthService"| p12
  p9 -->|"dim_app_server::managed_provider::ManagedProviders"| p12
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p12
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p11
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p10
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p8
```
