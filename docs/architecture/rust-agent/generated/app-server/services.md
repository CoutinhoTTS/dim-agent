# App Server 进程组合

由 `serve` 以产品配置（`--product-home`）真实装配生成，只含进程层，不含 Agent 实例。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph process["process"]
    p1["root"]
    p2["rutis-loader"]
    p22["Transport"]
    p21["Rpc"]
    p20["Connection"]
    p19["Collaboration"]
    p18["Agents"]
    p17["CollaborationDirectory"]
    p16["AgentDirectory"]
    p15["SkillsCatalog"]
    p14["HostPlatform"]
    p13["Mcp"]
    p12["Provider"]
    p11["GrokOAuth"]
    p10["OpenAICodexOAuth"]
    p9["ManagedProviders"]
    p8["Settings"]
    p7["SessionStore"]
    p6["SqliteStorage"]
    p5["Aimux"]
    p4["FileStorage"]
    p3["ServerControl"]
  end
  p20 -->|"dim_app_server::connection::ConnectionHub"| p22
  p3 -->|"dim_app_server::control::ServerControl"| p22
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p22
  p15 -->|"dim_agent_capabilities::skills::catalog::SkillsCatalog"| p21
  p18 -->|"dim_app_server::agents::AgentIndex"| p21
  p10 -->|"dim_app_server::codex_oauth::OpenAICodexOAuthService"| p21
  p19 -->|"dim_app_server::collaboration::Collaboration"| p21
  p20 -->|"dim_app_server::connection::ConnectionHub"| p21
  p3 -->|"dim_app_server::control::ServerControl"| p21
  p11 -->|"dim_app_server::grok_oauth::GrokOAuthService"| p21
  p9 -->|"dim_app_server::managed_provider::ManagedProviders"| p21
  p13 -->|"dim_app_server::mcp::McpService"| p21
  p12 -->|"dim_app_server::provider::ProviderService"| p21
  p8 -->|"dim_app_server::settings::SettingsService"| p21
  p3 -->|"dim_app_server::control::ServerControl"| p20
  p18 -->|"dim_app_server::agents::AgentIndex"| p19
  p17 -->|"dim_app_server::collaboration::CollaborationDirectory"| p19
  p16 -->|"dim_app_server::agents::AgentDirectory"| p18
  p8 -->|"dim_app_server::settings::SettingsService"| p18
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p18
  p7 -->|"dim_app_server::storage::timeline::SessionStore"| p18
  p2 -->|"rutis_loader::loader::Loader"| p18
  p14 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p15
  p8 -->|"dim_agent_capabilities::skill_settings::SkillSettingsHost"| p15
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p14
  p8 -->|"dim_app_server::settings::SettingsService"| p13
  p10 -->|"dim_app_server::codex_oauth::OpenAICodexOAuthService"| p12
  p11 -->|"dim_app_server::grok_oauth::GrokOAuthService"| p12
  p9 -->|"dim_app_server::managed_provider::ManagedProviders"| p12
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p12
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p11
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p10
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p8
  p6 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p7
```
