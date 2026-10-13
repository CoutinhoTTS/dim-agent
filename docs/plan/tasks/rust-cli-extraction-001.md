---
id: rust-cli-extraction-001
package: dimcode
module: extraction
status: completed
depends-on: []
---

# Rust CLI extraction

## objective

交付可独立构建的公共 Rust Agent/CLI、独立 TS OAuth 插件和默认禁用的构建/发布链路。两个仓库分别管理源码，公共历史不包含 Dim OAuth 实现。实际发布与 workflow 启用不在本任务范围内。

## context

- [项目剥离与发布方案](../analysis/rust-cli-extraction.md)。用户已授权按方案实施。
- 固定源提交：`3bc0cace829a144c2c9b4baae3fbddf0f5dd3ccb`，`feat/agent-v4`，2026-10-13 更新。
- 源码只有按该提交导出的 tracked Rust 文件；不导入构建缓存、用户目录或旧 Git 历史。
- 最新源实现采用 TypedPlugin 和进程 loader 行，沿用其装配、关闭与启动超时合同。
- 用户限定剥离阶段只向GitLab提交代码和MR；不提交、push或同步GitHub，后续同步需单独授权。

## path

公共仓库：`Cargo.toml`、`crates/`、`tests/`、`vendor/`、`xtask/`、`scripts/`、`npm/`、`.github/workflows/`、公开文档。

私有仓库：`nextim/dim-oauth-plugin` 的 TS Plugin、测试、打包脚本和文档。

## 调度

| 工作 | 执行者 | 隔离工作目录 | 输入与当前状态 |
| --- | --- | --- | --- |
| 公共导出、路径、文档、打包与集成 | 主 agent | `temp/dimrust-cli` | 公共源码、文档、打包及本机集成已验证 |
| Provider 解耦与真实 rutis 桥接 | public_provider | `temp/dimrust-provider-work` | 通用 Provider、真实 bridge、profile 身份及 v11 门禁已验证 |
| Rust CLI / scrollback | rust_cli | `temp/dimrust-cli-work` | Rust scrollback 与账号接线已验证，Windows compile 通过 |
| 私有 TS OAuth Plugin | private_oauth | `temp/dim-oauth-plugin` | 独立包、开发入口、实际跨仓模型流程已验证 |

主 agent 合入确定的产出，再由未参与对应模块开发的执行者独立验收。涉及同一 App Server 文件的修改由 public_provider 统一写入。源仓库只进行 fast-forward 更新，业务修改全部落在新仓库。

## verification

- 公共 workspace 的锁定依赖构建、相关 Agent/存储/Provider/CLI 流程测试、Clippy 与架构边界检查。
- 私有插件构建、实际 HTTP OAuth/store 流程及与真实 rutis runtime 的接线。
- 无私有目录的 API Key 路径、Rust scrollback 的输入/输出/审批/中断/恢复。
- 本机原生产 home 不改写；验收使用隔离目录。
- 实际本机 release/npm 打包，清单、摘要、运行资源与内容边界检查；发布工作流保持 `.disabled`。
- 跨平台代码/构建合同核对；无法在本机验证的平台明确记录，不冒称已通过真机验收。

## 交付结果

公共与私有实现、默认禁用的 CI/发布模板、本机打包和独立验收已完成，具体证据与未执行项见 [验收记录](../reviews/rust-cli-extraction-001.md)。

固定 Agent v4 源增量、宿主装配合同与 setup/run 文档的验证见 [源同步验收](../reviews/upstream-sync-001.md)。

- 公共 MR：https://gitlab.bfbops.com/nextim/dimrust-cli/-/merge_requests/2
- 私有 MR：https://gitlab.bfbops.com/nextim/dim-oauth-plugin/-/merge_requests/1
- 固定源工作区干净；业务修改仅在两个新仓库。
- GitHub同步、workflow启用与真实发布均未执行。其余四个平台原生验收在启用相应runner后完成，不宣称已通过。
