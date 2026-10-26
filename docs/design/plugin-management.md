# 插件管理：目标与待定义的需求

状态：需求定义草案，不是设计。本文写插件管理要达到什么目的、能带来什么、要先回答哪些问题；需求定下来之后，再修订或重写 [PluginManager](plugins/PluginManager.md) 等设计文档。现有机制只给出入口链接。

## 目的

让用户和项目能给 Dim 增加能力并管理它们：找到、安装、启用、更新、移除，并且始终清楚每个插件对 agent 做了什么、能动到什么。

这里有两类东西：

- **Dim 插件**：以 rutis 部件运行在 Dim 里的插件（Rust、TypeScript 或其他语言），可以注册工具、提示词来源、方法、服务，参与 Session 和分支。仓库自带的 Goal、Web、SessionTitle、Workflow、Scheduler 也属于这一类。
- **Claude Code / Codex 类的插件**：以 hooks、skills、命令、MCP 配置、agent 定义等内容为主的插件包，通常不带可执行的部件，靠外部脚本或宿主已有的机制生效。

## 能带来什么

- 用户：不改 Dim、不重启 daemon，就能装上第三方或第一方的能力，并且能关掉它、看清它。
- 项目：随仓库带上团队约定的插件，但在用户同意之前不生效。
- 插件作者：按[插件契约](plugin-contract.md)写好的插件，有一条到达用户手里的路径。
- 已经在用 Claude Code 或 Codex 的用户：已有的 hooks、skills 等投入能以较低成本带到 Dim。

## 要先回答的问题

1. **对用户来说，“插件”是一个概念还是两个？** Dim 插件和 Claude Code / Codex 类的插件，是用同一套安装、启用、列表来管理，还是分开？
2. **对 Claude Code / Codex 生态做到什么程度？** [Hooks](plugins/Hooks.md) 已经决定只读 Dim 自己的配置、格式保持兼容以便迁移，不直接加载对方的文件。插件包是否沿用这一立场，还是要能直接安装对方格式的插件包？
3. **作用域。** 插件装在哪一层：用户、项目、主机，还是节点？项目带来的插件怎样征得用户同意（[ADR 003](../adr/003-approval-scope.md)）？
4. **信任与可见性。** 用户在启用之前和之后需要看到什么：插件会注册哪些工具、会改哪些提示词、能执行什么命令、能访问哪些资源？
5. **生命周期。** 安装、启用、禁用、更新、移除各自对正在运行的 Session 有什么影响？更新需要用户确认吗？
6. **来源与发现。** 插件从哪里来：本地路径、Git、市场？需不需要市场，市场由谁维护？
7. **默认带什么。** 仓库自带的插件是否默认安装和启用？现在默认的 CLI 一个 TypeScript 插件都不加载（[#384](https://github.com/arcships/dim/issues/384)）。
8. **怎样算做成了？**

## 现有基础

- [PluginManager](plugins/PluginManager.md)：草案、P3，写于插件契约迁移之前；包格式 `dim-plugin.json`、市场 `dim-marketplace.json`、`pluginContents` 服务。
- [#384](https://github.com/arcships/dim/issues/384)：安装和启用第三方插件的范围（M2）。
- [#3](https://github.com/arcships/dim/issues/3)：loader 的可编辑层，作为启用状态的唯一来源。
- [Hooks](plugins/Hooks.md)：草案、Ready 候选；包来源的 hooks 留到 #3 之后。
- [ADR 004](../adr/004-loader-plugin-management.md)、[插件契约](plugin-contract.md)、[How Dim uses rutis](rutis-usage.md)。

需求跟踪：[#432](https://github.com/arcships/dim/issues/432)。
