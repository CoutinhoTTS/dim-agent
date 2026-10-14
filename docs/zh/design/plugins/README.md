# 插件设计

[总体设计](../overview.md)中规划的新插件的设计文档。已实现的插件及其 Capability 见 [architecture/rust-agent/plugins](../../../architecture/rust-agent/plugins/README.md)。规则：[设计规则](../../../architecture/rust-agent/principles.md) 第 1、4 节，[ADR 002](../../adr/002-plugin-scope.md)。共享合同：[session-access](../contracts/session-access.md)、[session-boundary](../contracts/session-boundary.md)。

插件按状态归属拆分：进程级部分是进程插件；Session 级和分支级部分是 Capability。所有文档状态：草案。

## 索引

| 插件 | 进程插件 | Session Capability | 分支 Capability | 阶段 |
| --- | --- | --- | --- | --- |
| [Questions](Questions.md) | — | `ask_user` 工具、`session.question.*` | 待答问题、`question/*` | P1 |
| [ShellInput](ShellInput.md) | — | `shell.*` 方法、命令执行 | — | P1 |
| [Attachments](Attachments.md) | — | `attachment.put`、Session 文件 | — | P1 |
| [SessionTitle](SessionTitle.md) | — | 首条输入观察、`session.rename` | — | P1 |
| [Goal](Goal.md) | — | `goal_update` 工具、`session.goal.*` | 目标状态、继续 | P2 |
| [Web](Web.md) | — | `web_search`、`web_fetch` 工具 | — | P2 |
| [Hooks](Hooks.md) | — | 规则、Session 开始 Hook | before 回调、工具调用后与 turn 结束 Hook | P2 |
| [Notifications](Notifications.md) | — | Session 观察、`notify` 工具 | — | P2 |
| [Scheduler](Scheduler.md) | 作业、触发、历史 | `schedule` 工具 | — | P2 |
| [Workflow](Workflow.md) | 定义、运行 | `workflow` 工具 | — | P2 |
| [ExternalAgent](ExternalAgent.md) | 连接、ModelHost 连接类型 | — | — | P3 |
| [AcpGateway](AcpGateway.md) | CLI 进程（客户端） | — | — | P3 |
| [RemoteGateway](RemoteGateway.md) | 远程入口、设备 | — | — | P3 |
| [Modality](Modality.md) | 任务、实时连接 | — | — | P3 |
| [PluginManager](PluginManager.md) | 插件市场、插件包 | — | — | P3 |

## 按作用域

| 作用域 | 部分 |
| --- | --- |
| 进程插件 | Scheduler、Workflow、ExternalAgent、Modality、PluginManager、RemoteGateway；AcpGateway 运行在 CLI 进程 |
| Session Capability | Questions、ShellInput、Attachments、SessionTitle、Goal、Web、Hooks、Notifications、Scheduler、Workflow |
| 分支 Capability | Questions、Goal、Hooks |
