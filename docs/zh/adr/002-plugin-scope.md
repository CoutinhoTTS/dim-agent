# 002：插件作用域划分与配置归属

决策状态：Proposed。适用于 `docs/design/` 中的插件设计及未来的插件。

## 背景

Dim 在三个作用域装配插件：进程、Session 和分支
（[单树装配](../../architecture/rust-agent/single-tree.md)）。插件的作用域决定它能依赖哪些服务、
能观察哪些事件、能写哪些事实，以及生命周期多长。

插件设计的早期版本为了减少 Capability 数量，把大部分新功能放到了进程作用域。这些功能仍需向
Session 提供工具、观察 Session 的事件并写入分支的输入，于是设计中加入了回连 Session 的接线：
扩展注册表、注册端口、Session 访问宿主、进程级 Session 事件订阅、进程级工具贡献、准入记录和
管理聚合插件。每一部分都只是为了连接放错作用域的插件而存在。

插件重载很少发生：只在结构性操作之后，例如安装、升级、启用或禁用插件，或崩溃。修改业务设置
不得触发任何重载。

## 决策

### D1. 作用域跟随状态及其生命周期的所有者

| 作用域 | 承载 | 现有示例 |
| --- | --- | --- |
| 进程 | 跨 Session 共享或比 Session 存活更久的资源与状态：连接、目录、存储、外部入口、跨 Session 的工作 | Provider, Mcp, SkillsCatalog, SessionStore, Transport |
| Session | Session 各分支共享的配置与 Capability：向 Session 提供的工具和提示词来源、对 Session 事件的观察、Session 自身的数据 | ToolRegistry, Skills, WorkspaceTools, TimelineTree |
| 分支 | 写入分支 Timeline 的事实、按序消费分支事件、绑定到某个 turn 的工作 | Loop, InputQueue, Approval, BackgroundExec |

只有当一个功能的各部分确实有不同所有者时，才把它拆到多个作用域，且每一部分恰好属于一个作用域。
没有跨 Session 共享状态的功能不是进程插件。

### D2. 不设接线模块

不存在只用于连接、聚合或适配其他插件的插件（注册表、端口、访问宿主、聚合或适配插件）。插件在
自身作用域通过现有机制集成：

| 需求 | 机制 |
| --- | --- |
| 向 Session 提供工具 | Session 级 `ToolContribution` 或 `SessionToolSource` |
| 提供指令 | PromptRegistry 提示词来源 |
| 观察 Session 的事件 | 现有的 Session 广播，由 Session 级插件监听 |
| 按序观察并响应分支事件 | 分支 Dispatcher，由分支级插件订阅 |
| 把内容放入模型上下文 | Session 或分支输入（被采纳的输入）、工具结果 |
| 在执行前检查工具调用 | 向 Approval 贡献的分支级准入检查 |
| 暴露关于某个 Session 的客户端方法 | Session 级方法贡献，由 Rpc 经该 Session 分发 |
| 暴露宿主管理方法 | 进程插件在 Rpc 方法表中的方法，标记为管理级；每个入口服务其配置的级别 |
| 访问其他 Session | Agents（AgentIndex）和 Session 公共接口，与 Collaboration 的做法相同 |
| 响应另一个进程插件的结果 | 事件通知，而非依赖 |

### D3. 依赖跟随作用域；接受罕见的重启

插件依赖自身作用域或外层作用域的插件。由于重载罕见且属结构性，重载传播到依赖方（并使在途 turn
失败）是这些操作的可接受代价；不为阻止它增加任何机制。易变运行时的崩溃隔离如果日后需要，属于
rutis（issue #6），而不属于应用层接线。

### D4. 结构性配置与业务配置分离

| 类别 | 示例 | 位置 | 变更时 |
| --- | --- | --- | --- |
| 结构性插件配置 | 启用或禁用、运行时与包路径、节点绑定、监听地址 | loader 行（issue #3 可编辑层） | 可能重载该行 |
| 业务配置 | 搜索服务、通知渠道、Hook 规则、定时任务、工作流定义、模型用途策略、MCP 服务器与 Skill 文档、外部 Agent 条目、exec shell | 所属插件的业务来源（Settings 来源或插件自有存储） | 数据更新并通知；使用时读取；不重载 |

## 新插件的作用域划分

| 插件 | 作用域 | 理由 |
| --- | --- | --- |
| Questions | Session（`ask_user` 工具）+ 分支（待答问题、`question/*` 事实） | 问题属于某个分支的 turn |
| Goal | Session（`goal_update` 工具）+ 分支（目标状态、续跑） | 目标属于某个分支，续跑按序跟随其事件 |
| ShellInput | Session | 命令在 Session 的工作区中运行并写入其输入 |
| Attachments | Session | 附件是 Session 数据 |
| Web | Session | 没有跨 Session 共享的状态；只提供工具 |
| SessionTitle | Session | 观察 Session 的首条输入；标题由 SessionStore 保存 |
| Notifications | Session | 观察 Session 的事件并提供 `notify` |
| Hooks | Session（规则、Session 启动 Hook）+ 分支（工具前、工具后与 turn 结束 Hook；向 Approval 贡献准入检查） | 规则作用于 Session；执行发生在分支 |
| Scheduler | 进程（任务、触发器）+ Session（`schedule` 工具） | 调度跨越多个 Session |
| Workflow | 进程（运行）+ Session（`workflow` 工具） | 一次运行跨越多个 Session |
| ExternalAgent | 进程 | 外部 Agent 连接是共享的 |
| Modality | 进程 | 任务与 provider 连接是共享的 |
| PluginManager | 进程 | 安装以宿主为单位 |
| RemoteGateway | 进程 | 一个外部入口 |
| AcpGateway | CLI 进程 | 一个客户端 |

管理不是插件：管理方法以带级别的行存在于 Rpc 的方法表中，由各入口按级别提供服务。

## 撤回

撤回以下内容：扩展注册表合同、注册端口及其宿主插件、Session 访问宿主、进程级 Session 事件订阅、
进程级工具贡献、准入记录（`RequiredCheck`）以及 Management 插件。

## 现有插件

现有插件已遵循 D1。有一项配置不符合 D4：exec shell 在 HostPlatform 的 `apply` 中只读取一次；
它应作为业务配置在每次 spawn 时读取。

## 影响

- 设计文档按此划分修订：总览、合同（仅保留描述现有插件补充的合同）以及每个插件文档。
- 可选的 Session 与分支插件只在启用时安装；它们不是每次装配中的占位。
- 罕见的结构性操作可能使在途 turn 失败；这一点被记录，而不被阻止。
