# 插件设计

[总体设计](../overview.md)中规划的新插件的设计文档。已实现的插件及其 Capability 见 [architecture/rust-agent/plugins](../../../architecture/rust-agent/plugins/README.md)。规则：[设计规则](../../../architecture/rust-agent/principles.md) 第 1、4 节，[ADR 002](../../adr/002-plugin-scope.md)。共享合同：[session-access](../contracts/session-access.md)、[session-boundary](../contracts/session-boundary.md)。

插件按状态归属拆分：进程级部分是进程插件；Session 级和分支级部分是 Capability。所有文档状态：草案；ShellInput 与 Attachments 已实现，Hooks 待设计，Notifications 已撤回。

## 索引

| 插件 | 进程插件 | Session Capability | 分支 Capability | 阶段 |
| --- | --- | --- | --- | --- |
| [Questions](Questions.md) | — | `ask_user` 工具、`session.question.respond` | 运行中 `ask_user` 调用的等待者；没有事件 | M2 |
| [ShellInput](ShellInput.md) | — | `shell.*` 方法、命令执行 | — | P1（已实现） |
| [Attachments](Attachments.md) | — | `attachment.put`、Session 文件 | — | P1（已实现） |
| [SessionTitle](SessionTitle.md) | — | 观察首条用户输入的采纳、`session.rename` | — | P1 |
| [Goal](Goal.md) | — | `goal_update` 工具、`session.goal.*` | 由 `goal:objective` 与 `goal:status` 块折叠出的目标状态、续跑 | P2 |
| [Web](Web.md) | — | `web_search`、`web_fetch` 工具 | — | P2 |
| [ContextCompaction](ContextCompaction.md)（Loop 与 ContextPreparation 的补充） | — | `ContextModelHost`、摘要 `CompactionStrategyHost`、`session.context.*` | Loop 中的 `context/compact` 指令与压缩步骤；ContextPreparation 中的 K 轮切点与用量查询 | P2 |
| [Hooks](Hooks.md) | — | 规则、Session 开始 Hook | before 回调、工具调用后与 turn 结束 Hook | 待设计 |
| [Notifications](Notifications.md) | — | Session 观察、`notify` 工具 | — | 已撤回 |
| [Scheduler](Scheduler.md) | 触发、运行历史（任务在 Settings 中） | `schedule` 工具 | — | P2 |
| [Workflow](Workflow.md) | 运行（定义在 Settings 或项目文件中） | `workflow` 工具 | — | P2 |
| [ExternalAgent](ExternalAgent.md) | 连接、`modelKind` `external-agent`、询问；每节点一个桥接行 | 询问决定工具、`external_agent` 方法 | — | P3 |
| [AcpGateway](AcpGateway.md) | —（CLI 进程中的客户端，不是守护进程插件） | — | — | P3 |
| [RemoteGateway](RemoteGateway.md) | 远程入口、设备 | — | — | P3 |
| [Modality](Modality.md) | 任务、实时连接 | — | — | P3 |
| [PluginManager](PluginManager.md) | 插件市场、插件包 | — | — | P3 |

## 按作用域

| 作用域 | 部分 |
| --- | --- |
| 进程插件 | Scheduler、Workflow、ExternalAgent、Modality、PluginManager、RemoteGateway；AcpGateway 不是进程插件，而是 CLI 进程中的客户端 |
| Session Capability | Questions、ShellInput、Attachments、SessionTitle、Goal、Web、Hooks、Scheduler、Workflow、ExternalAgent |
| 分支 Capability | Questions、Goal、Hooks |
