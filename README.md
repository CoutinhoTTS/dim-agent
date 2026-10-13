# Dim Rust CLI

Dim 的 Rust 命令行项目，采用 Agent v4 核心和逐步演进的 scrollback 交互。

| 项目 | 约定 |
| --- | --- |
| 开发与评审仓库 | `nextim/dimrust-cli` |
| 开源目标 | [arcships/dim](https://github.com/arcships/dim) |
| npm 包与命令 | `dimcode`；`dim` / `dimcode` |
| 公共源码许可证 | MIT |
| Dim OAuth | 独立的闭源 TypeScript rutis Plugin |

当前仓库包含设计与交付方案，以及默认禁用的 [npm 发布工作流](.github/workflows/npm-publish.yml.disabled)和产物校验脚本。源码提取、Rust CLI、插件接线和构建工作流按方案实施。发布工作流尚未注册或启用。

完整范围、模块合同、实施任务和验收条件见[项目剥离与发布方案](docs/plan/analysis/rust-cli-extraction.md)。
