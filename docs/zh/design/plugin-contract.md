# 插件契约

[English](../../design/plugin-contract.md)

状态：已接受，见 [ADR 006](../adr/006-plugin-contract.md)。尚未整体实现。各节说明今天已成立的部分。
它所依据的理论，即[时间总线](../../architecture/rust-agent/time-bus.md)与[设计规则](../../architecture/rust-agent/principles.md)，
不变，现在即适用。

本文是 Dim 的架构落到插件上的部分，不论用哪种语言编写：实例树的含义、部件放在哪里、Dim 定义的服务与
总线事件、Timeline 事实及其消费、总线事件如何消费、Capability，以及发布与关闭。部件如何装载、如何命名服务、如何
跨语言、如何使用 rutis 的生命周期和总线，见 [Dim 如何使用 rutis](rutis-usage.md)，它的附录是 Rust 与
TypeScript 的绑定。绑定不增加语义。若某个绑定看似需要一条两份文档都没有写明的规则，先修改文档。

## 阅读路径

| 你在做 | 阅读 |
| --- | --- |
| 编写或评审任何插件 | [时间总线](../../architecture/rust-agent/time-bus.md)，然后本文 §1–§8，然后 [Dim 如何使用 rutis](rutis-usage.md) 及你所用语言的附录 |
| 设计插件 | 另读 [ADR 002](../adr/002-plugin-scope.md)（位置、配置）和 §6（哪些规则通常适用） |
| 修改机制（Timeline、Dispatcher、装配、服务目录） | 另读[原则 §4 第 8 条](../../architecture/rust-agent/principles.md)和 §9 |
| 改动现有部件 | 它在 [`plugins/`](../../architecture/rust-agent/plugins/README.md) 或 [`design/plugins/`](plugins/README.md) 下的页面 |

[single-tree](../../architecture/rust-agent/single-tree.md) 与 [lifecycle](../../architecture/rust-agent/lifecycle.md)
文档描述当前 Rust 实现如何落实本契约。编写插件不必阅读它们。

## 1. 概念

| 术语 | 含义 |
| --- | --- |
| 实例树 | 进程根，以及其下的 Agent、Session 和分支实例（§2） |
| 部件 | 作为一个 loader 行装载的一个 rutis 插件（[rutis 使用 §2](rutis-usage.md#2-部件是-loader-行)）。一个功能（例如一个插件包）通常有几个位于不同作用域的部件 |
| 作用域 | 部件所在的位置：进程、某个 Session 或某条分支（§2、§3）。rutis 用实例化的 loader 组实现每个作用域（[rutis 使用 §2](rutis-usage.md#2-部件是-loader-行)） |
| 服务 | 一个部件向其他部件提供的命名对象；它的标识是一个名称和一个作用域（[rutis 使用 §3](rutis-usage.md#3-按名字的服务)） |
| 服务目录 | Dim 定义的名称（§4） |
| Timeline | 一条分支有序、持久的事实日志 |
| 事实 | 已提交的 Timeline 事件：`AgentEvent` 信封（`id`、`sequence`、创建时间、可选的 `tags`，以及 `type`、`name`、`data`） |
| 领域 | 事件的 `type`。核心保留自己的领域；扩展只写自己的领域（§5.1） |
| 指令 | 要求某个状态的所有者修改该状态的事实或输入。所有者决定并写下结果 |
| Dispatcher | 分支的机制，按序号顺序把已提交的事实投递给在它上面注册的部件 |
| `before` | 在提交锁内对草稿事实的同步变换。仅限 Rust、仅限进程内（§5.1） |
| 总线事件 | 根的平总线上的 rutis 事件（[rutis 使用 §5](rutis-usage.md#5-事件总线)）；Dim 的部件如何消费总线事件见 §5.3。不要与 Session 通知（Session 广播上的一种载荷）混淆 |
| 代次 | 部件的一次装载，从 `apply` 到卸载（[rutis 使用 §4](rutis-usage.md#4-rutis-提供的生命周期)）。重载开始新的代次 |
| Capability | 分支 Timeline 上的状态机部件（§6） |
| 所有者 | 写某块状态或某个领域的部件。每个状态恰好有一个所有者 |

## 2. 实例树

```text
process (root)                 process parts: storage, catalogs, connections, Agents, Rpc …
└─ agent ⟨instanced⟩          the host's Agent facade; no feature parts
   └─ session ⟨instanced⟩     one Session: its Session parts and Session services
      └─ branch ⟨instanced⟩   one branch: its Timeline, Dispatcher and branch parts
```

| 作用域（rutis 组） | 实例打开于 | 关闭于 | 实例中的机制服务 |
| --- | --- | --- | --- |
| 进程（根） | 宿主启动 | 宿主关闭 | —（进程部件自行提供） |
| Agent（`agent`） | Agents 创建一个 Agent | Agent 关闭、宿主关闭 | 宿主内部 |
| Session（`session`） | 某个 Agent 创建或恢复一个 Session | Session 关闭、空闲卸载 | `sessionIdentity`、`sessionLifecycle`、`treeCommands` |
| 分支（`branch`） | TimelineTree 创建一条分支 | 分支结束，或其 Session 关闭 | `timeline`、`dispatcher`、`branchLifecycle`、`timelineIdentity` |

可见性沿树，与[原则 §2](../../architecture/rust-agent/principles.md) 一致：

| 方向 | 方式 |
| --- | --- |
| 向上（外层实例、进程） | 按名称注入服务 |
| 向下（Session 到分支） | `treeCommands`：按分支 id 输入、创建分支。从不通过注入分支服务 |
| 横向（兄弟分支、其他 Session） | 不可见。经共同祖先的业务服务，例如 `sessions` |

作用域根、门面及其发布规则属于机制。每个 Session 和分支只在其装配 profile 要求的行都处于 Active
之后才发布（[single-tree](../../architecture/rust-agent/single-tree.md#结构)）。

*现状：*树、作用域和可见性规则已实现。`sessionIdentity` 和 `sessionLifecycle` 只作为 Rust 服务存在
（需求 R2）。

## 3. 位置

部件的位置就是它的作用域；rutis 把它实现为部件所在行的组（[rutis 使用 §2](rutis-usage.md#2-部件是-loader-行)）。放在哪个作用域，由 Dim 决定：

1. **按状态的所有者放置**（[ADR 002 D1](../adr/002-plugin-scope.md#d1-作用域跟随状态及其生命周期的所有者)）。
   跨 Session 共享或比 Session 存活更久的状态放在进程。Session 各分支共享的状态放在 Session。
   分支的事实、它的有序消费和绑定到 turn 的工作放在分支。只在功能的各部件确实有不同所有者时
   才拆分它，并在一份设计中一起描述这些部件。
2. **必需还是可选由宿主决定。** 装配 profile 列出 Session 或分支要等待的行。由配置添加的行是可选的：
   它处于 Pending 或 Failed 时，只影响它自己的功能。
3. **结构性配置与业务配置分开**（[ADR 002 D4](../adr/002-plugin-scope.md#d4-结构性配置与业务配置分离)）。
   业务设置存在所有者的来源中，使用时经 `settings` 读取。修改它们不重载任何东西。

## 4. Dim 的服务

下表是 Dim 定义的名称，每个都遵循 [rutis 使用 §3](rutis-usage.md#3-按名字的服务) 的标识与形状规则。
`xtask` 将从真实装配生成权威的表（rutis 使用 R4）。在此之前，以这张表、`crates/dim-app-server/build.rs`
和[作者指南](../../typescript-plugin-authoring.md#available-services)为准。

| 名称 | 作用域 | 提供者 | 今天是否跨语言 | 说明 |
| --- | --- | --- | --- | --- |
| `sessions` | 进程 | Agents | 是 | 启动或驱动另一个 Session |
| `sessionPermissions` | 进程 | Agents | 是 | 生成的导出 |
| `sessionMetadata` | 进程 | SessionStore | 是 | SessionTitle 是标题的唯一写入方 |
| `settings` | 进程 | Settings | 是 | 业务配置 |
| `auxiliaryModel` | 进程 | App Server | 是 | 辅助模型请求 |
| `pluginFiles` | 进程 | 宿主 | 是，当宿主提供根目录时 | 进程部件自己的文件 |
| `workflowFiles` | 进程 | App Server | 是 | 生成的导出 |
| `managementMethods:<namespace>` | 进程 | 所有者 | 是 | 仅限 protected-local v3 |
| `collaborationMembers:<kind>` | 进程 | 成员提供者 | 已设计，未实现 | [ExternalAgent](plugins/ExternalAgent.md) |
| `treeCommands` | `session` | TimelineTree | 是 | 按分支 id 的 `input`、`create`、`list`、`info` |
| `toolContribution:<tool>` | `session` | 该工具的部件 | 是 | 由 ToolRegistry 消费；今天有一个原生标识和一个外来标识（rutis 使用 R6） |
| `sessionMethods:<namespace>` | `session` | 该命名空间的部件 | 是 | 由 Rpc 分发 |
| 提示词来源、工具来源 | `session` | 各自的部件 | 仅限 Rust | 今天由宿主预先注册（rutis 使用 R6） |
| `sessionIdentity` | `session` | SessionScope | 仅限 Rust | `{sessionId, restored}`；R2 把它作为数据暴露给每种语言 |
| `sessionLifecycle` | `session` | SessionScope | 仅限 Rust | `activated`、`closing`、`hold`；R2 把它暴露给每种语言 |
| `sessionEvents:<part>` | `session` | C4 的 setup | 是 | 从 0.16 升级起只用于绑定行的 Session id；各行自己监听 `session/{sessionId}`。由 R2 取代 |
| `timeline` | `branch` | BranchScope | 是，`before` 除外 | 部件自己领域的 `append`；历史读取 |
| `dispatcher` | `branch` | BranchScope | 是，`listen` 和读取；`subscribe` 仅限 Rust | 有序消费（§5.2） |
| `branchLifecycle` | `branch` | BranchScope | 是 | `activated`、`closing` |
| `timelineIdentity` | `branch` | BranchScope | 是 | `{sessionId, branchId}` |

Rust 宿主端口和注册表（ToolRegistry、PromptRegistry、ToolHost、ModelHost、PrepareModelHost，
以及[生成的服务](../../architecture/rust-agent/generated/services.md)中的其他项）在阶段 2 获得名称和组。
除非外来部件需要它们，它们保持仅限 Rust。`BranchOpening`（开启回执是 Rust future）和门面的状态读取
（`StateCell<S>`、`PublicService`）同样如此，它们以 Rust 类型为 key。外来部件通过服务暴露自己的状态。

两条机制契约把门面绑定到核心部件：分支只在有 `LoopState` 提供者时发布，Session 只在有
`TimelineTreeControl` 提供者时发布。因此 Loop 和 TimelineTree 是核心部件，用 Rust 编写。
本设计不改变这一点。

## 5. 事件

### 5.1 Timeline 事实

部件经所在分支的 `timeline` 服务写事实。Timeline 只检查信封，并拒绝在核心领域中的不透明追加
（`dim-agent-types` 中的 `RESERVED_DOMAINS`：`input`、`stop`、`approval`、`permission`、`exec`、`turn`、
`prompt`、`context`、`model`、`step`、`branch`、`collaboration`、`session`）。扩展写自己的领域，例如它的
插件 id；`type` 和 `name` 非空，且不含 `/` 或 `*`（[ADR 001](../../adr/001-harness-foundation.md#events-and-ownership-issue-2)）。

写事实的规则这里不再复述。向所有者写指令、交接、`before`、批次、模型上下文的路径，以及哪些事实
应进入 Timeline，见[原则 §4](../../architecture/rust-agent/principles.md) 第 1–3、5、6、9 条和 AGENTS
第 10 条。其中一条推论对其他语言有影响：`before` 变换在提交锁内运行，而提交临界区内不得运行任何
跨进程代码（[ADR 001](../../adr/001-harness-foundation.md#trust-and-cross-language-calls)），所以只有进程内的
Rust 能注册它。

### 5.2 消费事实

部件在分支的 `dispatcher` 上注册，写明它消费的键：精确的 `domain/name`、`domain/*` 或 `*`，以此消费
该分支的事实。Dispatcher 按序号顺序逐条投递，本代次注册的每个部件都会收到。新代次按
[能力重载协议](../../architecture/rust-agent/single-tree.md#能力重载协议)重建。

### 5.3 总线事件

总线事件是根的平总线上的 rutis 事件；它的投递、载荷与命名规则见 [rutis 使用 §5](rutis-usage.md#5-事件总线)。
消费总线事件同样是消费，只是投递比事实弱：总线事件不存储、不回放，只有发出时已经注册的监听器才收到。因此通知的消费方按[时间总线 §3](../../architecture/rust-agent/time-bus.md) 对每个状态机的
要求恢复：从 Timeline 恢复，或重新读取所有者的状态。这样漏掉一条总线事件只会推迟某个决定，不会改变它。
[SessionTitle](plugins/SessionTitle.md) 就是一例：每条 `input/applied` 事件都让它重新读取标题元数据和
固定的历史前缀，所以漏掉一条事件只会把标题推迟到下一条被采纳的输入。

Dispatcher 和 Timeline 内部使用的总线名字归它们自己；任何部件都不在其下发送或监听。

公开总线事件：

| 名称 | 发送方 | 载荷 | 含义 |
| --- | --- | --- | --- |
| `session/{sessionId}` | SessionBroadcast | 一条已提交的事件、一条 Session 通知、一个 fence，或 closed | Session 面向客户端和观察者的实时投影（[Session 广播](../../architecture/rust-agent/event-driven-flows.md#6-session-广播)） |
| `settings/{source}` | Settings | `{source, scope, revision}` | 某个业务来源已变化 |
| `settings/projects` | App Server Settings | `{root, cwd}` | 某个项目根已授权 |
| `skills/catalog` | SkillsCatalog | 目录版本 | Skill 目录已推进 |
| `agent/session-restored`、`agent/unloaded-session-deleted` | Agents | id | 宿主内部，由 Collaboration 消费 |

*现状：*这些事件已按上表实现，TypeScript 行直接监听它们。

## 6. Capability

Capability 是按序号从分支的事实折叠状态，或写自己领域的事实的部件，即分支 Timeline 上的状态机。它定义在
[原则 §1](../../architecture/rust-agent/principles.md)；不声明，也没有任何机制强制
（[ADR 006 D6](../adr/006-plugin-contract.md#d6-capability-是约定分支-timeline-上的状态机)）。只提供服务或只写输入的
部件不是 Capability，就是部件。

是否为 Capability 本身不增加规则；它只说明哪些现有规则通常适用：

| 部件 | 通常适用的现有规则 |
| --- | --- |
| Capability | [时间总线](../../architecture/rust-agent/time-bus.md) §1–§5；原则 §4 第 1–3、5、6、9 条；AGENTS 第 3、8 条 |
| 所有部件 | ADR 002 D1–D5；原则 §4 第 1、2、7、8 条 |

示例：

- Goal 的分支部件折叠事实并决定是否续写，所以是 Capability。
- Goal 的 Session 部件提供 `goal_update` 和 `session.goal.*`，并按分支 id 写变更输入。它不持有 Goal 状态，
  所以不是 Capability。
- Questions 提供 `ask_user` 和 `session.question.*`。它的等待者是它自己的工具调用，它不写事实，所以不是
  Capability。

设计文档的规则检查写明哪些部件是 Capability，然后按 AGENTS.md 的要求回答原有清单：原则 §4 第 1–9 条、
§1 和 ADR 002 D1–D5。上表只说明哪些规则通常适用；不适用的规则也要写明原因。

## 7. 部件的生命周期

### 7.1 阶段

构建、`apply` 和卸载是 rutis 的（[rutis 使用 §4](rutis-usage.md#4-rutis-提供的生命周期)）。
Dim 在它们之间为 Session 部件和分支部件增加两个阶段：

| 阶段 | 部件可以 | 机制做什么 |
| --- | --- | --- |
| 发布（`activated`） | 启动后台工作和输入消费 | Session 或分支发布时 `activated()` 完成；装配失败时为 `false` |
| 关闭（`closing`） | 停止准入、取消自己的工作、释放资源、写终态事实 | 关闭开始时 `closing()` 完成。Session 从开始关闭的那一刻起就处于关闭中 |

Dispatcher 处理函数可能在发布（`activated`）之前收到开启阶段的事实。

### 7.2 代次、重载与重建

重载是从 Timeline 重新开始（时间总线 §5）。正在卸载的代次如何结算自己的工作，新代次如何订阅、固定截点、
重建，以及如何让找到的未完成工作失败，由[能力重载协议](../../architecture/rust-agent/single-tree.md#能力重载协议)
规定。它适用于每种语言；R1 把其中订阅与重建的步骤变为一次 Dispatcher 注册。

## 8. 保证、约定与审计

机制给出的保证列在 [ADR 006 D7](../adr/006-plugin-contract.md#d7-机制保证什么不保证什么) 中。设计需要回答的规则
（§6）都是约定。有三样东西检查它们：

| 检查 | 覆盖什么 |
| --- | --- |
| `xtask graph` / `check` | 现状：crate 边界、生成的装配图，以及插件页面与代码的对照。rutis 使用 R4 之后，它还写出服务目录表，并标出哪些部件是 Capability（它们注入 `dispatcher` 或 `timeline`） |
| 评审与 PR 模板 | 规则检查：原则 §4 第 1–9 条、§1、ADR 002 D1–D5 |
| 流程测试 | 可观察的行为：顺序、重建、重载结算、关闭 |

## 9. 向机制所有者提出的需求

每项需求来自机制自身的职责，并在机制内闭环（[原则 §4 第 8 条](../../architecture/rust-agent/principles.md)）。
本设计合并后再开 issue。R4–R6 关于 Dim 如何使用 rutis，见
[rutis 使用 §6](rutis-usage.md#6-向机制所有者提出的需求)。

### R1. 有序消费与重建合为一次 Dispatcher 注册

- **受阻：**[能力重载协议](../../architecture/rust-agent/single-tree.md#能力重载协议)中订阅与重建的步骤。每个 Capability 都手写一遍。例子有
  `crates/dim-agent-capabilities/src/wait.rs:361-380` 和 `runtime/plugins/goal/branch.ts:120-128`。
  一个错误（在订阅之前读取截点、用 `history` 代替 `local`）会无声地丢失或重复计算事件。
- **考虑过的现有机制：**`subscribe`/`listen` 加 `committed`/`local` 是正确的，但把协议留给每个作者。
  每种语言一个 SDK 辅助函数会复制这个协议（ADR 006 D1）。
- **所需机制：**`dispatcher.consume(name, keys, handler)`，或 `listen` 上的一个选项：它注册、固定 `S`、
  以 `replay: true` 投递 `local(..=S)`，然后投递 `S` 之后的实时事件，通过同一个处理函数，按序号顺序。
  它不带业务含义。
- **对序号、批次、中断、重载的影响：**对 Timeline 没有影响。投递顺序与手写协议的相同。重载行为即 §7.2 所述。
- **所有者：**Dim 运行时（Dispatcher）。issue 待开。

### R2. Session 作用域与分支作用域、每种语言都有生命周期与标识服务

- **受阻：**AGENTS 第 1 条（后台工作在发布之后才开始）。TypeScript Session 行没有 `activated()` 或 `closing()`，而 Rust Session 部件有
  `SessionLifecycle`（[作者指南](../../typescript-plugin-authoring.md#plugin-shape-and-lifetime)）。
  该行从 C4 的 setup 而不是从服务得知自己的 Session id。
- **考虑过的现有机制：**C4 的逐行 setup 带有 id，但不带阶段。分支行已有 `branchLifecycle`。
- **所需机制：**`sessionLifecycle`（`activated`、`closing`、`hold`）和 `sessionIdentity`
  （`{sessionId, restored}`）作为 `session` 中的服务目录服务，带 serde 形状，与 `branch` 中已有的
  `branchLifecycle` 和 `timelineIdentity` 并列。`hold` 返回一个句柄，由一次调用或该行的卸载释放。进程作用域没有：
  进程部件经 rutis 生命周期跟随宿主的启动与关闭。
- **影响：**对 Timeline 没有影响。它暴露现有的阶段。
- **所有者：**Dim 运行时（SessionScope）和 App Server（服务目录）。issue 待开。

### R3. Rpc 把贡献方法的服务错误映射为 wire 错误

- **受阻：**ADR 002 D5（错误即数据）。`runtime/plugins/goal/session.ts:15` 把原生错误变体（`InvalidInput`、`UnknownBranch`、
  `Closed`、`Reloading`）映射为 JSON-RPC `appError` 码。每个转发原生错误的方法提供者都得重复这个映射，
  而新增一个原生错误码需要修改每个插件。
- **考虑过的现有机制：**`MethodReply` 的错误形式 `{code, appError, message}` 把 wire 错误码放进了插件契约。
- **所需机制：**贡献的方法返回 `{code, message}` 数据，或重新抛出它收到的服务错误。Rpc 把原生的和
  有文档的错误码映射为 wire `appError`，与它对内置方法已有的做法相同。
- **影响：**对 Timeline 没有影响。wire 结果保持不变。
- **所有者：**App Server（Rpc）。issue 待开。它改变 TypeScript SDK 的形状，因此在阶段 4 落地。

## 10. 迁移

每个阶段都是一次完整、可评审的改动，带有它的测试和文档。Dim 尚未发布，所以任何阶段都不保留兼容路径：
不为仓库外的任何人留别名、垫片或过渡形态。某个阶段需要时，就同时修改 SDK、仓库内的插件和文档。
本设计不改变任何持久化格式或 wire 格式。R1–R3 在 §9，R4–R6 在 [rutis 使用 §6](rutis-usage.md#6-向机制所有者提出的需求)。

| 阶段 | 工作 | 验收 |
| --- | --- | --- |
| P1 | [rutis 0.16 升级](rutis-0.16-upgrade.md) | 该设计的流程检查和完整的 CI 序列 |
| P2 | 服务目录与 Rust 绑定：注册 Dim 的名称和组；Rust 部件按名称声明依赖，从服务目录及其实例链取得 key（[rutis 使用附录 A](rutis-usage.md#附录-a-rust-绑定)）；旧类型只保留到 P3 迁完所有部件为止；R4 的服务目录表 | 不导入任何层类型的 Rust 部件可以在进程、`session` 和 `branch` 中挂载；放错位置的行在组合时失败，并给出它需要的组 |
| P3 | 把每个部件迁到使用名称依赖的行，分四批：进程与简单部件；核心分支部件；核心 Session 部件；App Server 宿主部件。R6：带前缀服务的提供者按名称查找。每一批都按功能检查同一作用域里的部件：只有一个所有者、又没有分开的理由（例如重载隔离）的就合并；合并单独提交，和迁移分开。删除 `Capability` trait、作用域类型、`In*`/`*Gate` 包装、`*Service` 标记、`session_plugin`/`timeline_plugin` 和名称预先注册。重新生成插件页面 | `xtask check` 通过；代码和文档中不再有层类型；每次合并都单独提交；现有的多 Session 与多分支流程测试、顺序、取消和门面限制不变 |
| P4 | 跨语言对等：R2、R3 和 R5；用 `#[rutis::service]` 导出 Dim 的跨语言服务；外来运行时的输出进入进程日志，而不是被丢弃（[ADR 005](../../adr/005-observability.md)）；SDK 与作者指南随之修改；私有的 managed-provider 插件随后跟进 | TypeScript 行和 Python 行各自无需宿主推断即可在 `session` 和 `branch` 中挂载、消费事实、提交输入并提供一个工具。基于 Node 的流程测试通过 |
| P5 | R1 | 每个 Capability 重建测试以一次注册的形式通过；手写形式仍然通过 |
| P6 | 指导与生成文档定稿：插件页面、作者指南、single-tree 和 lifecycle 描述已实现的契约 | `xtask check`；没有文档要求不存在的 API |

issue 拆分，在本设计合并时替换 #370–#376：

| 现有 | 变为 |
| --- | --- |
| #370（设计） | 由本设计关闭 |
| #371（`Service<T>` 与原始挂载 API） | P2，不含 `Service<T>`：服务目录注册与名称依赖 |
| #372、#373、#374（迁移） | P3，同样的三批，外加进程与简单部件 |
| #375（删除旧 API） | P3 的结尾 |
| #376（其他语言的原始行） | P4（R2、R3、R5） |
| #362（`SkillsCatalog` watch） | 关闭：在平总线上，该事件到达每个 Session 行 |
| 新开 | P1（0.16 升级）、P5（R1）、R4、R6 |

## 11. 规则检查

本设计改变的是插件基础设施的契约，而不是某个业务插件。

| 规则 | 回答 |
| --- | --- |
| 原则 §1（插件与 Capability 有界） | Capability 收窄为分支 Timeline 上的状态机（§6）。不新增接线模块；App Server 的中继被移除（0.16 升级 U5），组推断也被移除（rutis 使用 R5） |
| §4 第 1 条（指令） | 不变 |
| §4 第 2 条（交接） | 不变 |
| §4 第 3 条（`before`） | 不变；现在写明仅限进程内（§5.1） |
| §4 第 4 条（核心不带业务含义） | 服务目录、R1 和 R3 是通用的；不新增针对特定插件的处理 |
| §4 第 5 条（批次） | 不变 |
| §4 第 6 条（上下文） | 不变 |
| §4 第 7 条（只通过公开内容交互） | 不变 |
| §4 第 8 条（基础设施默认不变） | R1–R6 各自来自机制自身的职责，并在其内闭环；rutis 按现状使用 |
| §4 第 9 条（只写需要的事实） | 不新增事实 |
| ADR 002 D1–D5 | D1–D4 不变；D5 补全：一个命名服务目录，注册由插件声明（R6），Rust 作为一种绑定 |
