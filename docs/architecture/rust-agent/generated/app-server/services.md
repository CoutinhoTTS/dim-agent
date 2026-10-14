# App Server 进程组合

由 `serve` 以产品配置（`--product-home`）真实装配生成，只含进程层，不含 Agent 实例。重新生成：`cargo run -p xtask -- graph`。

```mermaid
%% Generated from live rutis diagnostics; do not edit.
flowchart LR
  subgraph process["process"]
    p1["root"]
    p2["rutis-loader"]
    p24["Transport"]
    p23["Rpc"]
    p22["Connection"]
    p21["Collaboration"]
    p20["Agents"]
    p19["CollaborationDirectory"]
    p18["AgentDirectory"]
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
  p22 -->|"dim_app_server::connection::ConnectionHub"| p24
  p3 -->|"dim_app_server::control::ServerControl"| p24
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p24
  p20 -->|"dim_app_server::agents::AgentIndex"| p23
  p21 -->|"dim_app_server::collaboration::Collaboration"| p23
  p22 -->|"dim_app_server::connection::ConnectionHub"| p23
  p3 -->|"dim_app_server::control::ServerControl"| p23
  p9 -->|"dim_app_server::methods::ManagementMethods#account"| p23
  p10 -->|"dim_app_server::methods::ManagementMethods#account.codex"| p23
  p11 -->|"dim_app_server::methods::ManagementMethods#account.grok"| p23
  p15 -->|"dim_app_server::methods::ManagementMethods#mcp"| p23
  p12 -->|"dim_app_server::methods::ManagementMethods#provider"| p23
  p17 -->|"dim_app_server::methods::ManagementMethods#skills"| p23
  p12 -->|"dim_app_server::provider::ProviderService"| p23
  p3 -->|"dim_app_server::control::ServerControl"| p22
  p20 -->|"dim_app_server::agents::AgentIndex"| p21
  p19 -->|"dim_app_server::collaboration::CollaborationDirectory"| p21
  p12 -->|"dim_agent_capabilities::host::ModelSelectionHost"| p20
  p18 -->|"dim_app_server::agents::AgentDirectory"| p20
  p13 -->|"dim_app_server::settings::SettingsService"| p20
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p20
  p8 -->|"dim_app_server::storage::timeline::SessionStore"| p20
  p2 -->|"rutis_loader::loader::Loader"| p20
  p16 -->|"dim_agent_capabilities::host_platform::HostPlatform"| p17
  p13 -->|"dim_app_server::settings::SettingsService"| p17
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p16
  p14 -->|"dim_app_server::mcp::publication::Publication"| p15
  p13 -->|"dim_app_server::settings::SettingsService"| p15
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p14
  p13 -->|"dim_app_server::settings::SettingsService"| p14
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p13
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p13
  p10 -->|"dim_app_server::codex_oauth::OpenAICodexOAuthService"| p12
  p11 -->|"dim_app_server::grok_oauth::GrokOAuthService"| p12
  p9 -->|"dim_app_server::managed_provider::ManagedProviders"| p12
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p12
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p11
  p4 -->|"dim_agent_capabilities::file_storage::FileStorage"| p10
  p7 -->|"dim_app_server::storage::sqlite::SqliteStorage"| p8
```
