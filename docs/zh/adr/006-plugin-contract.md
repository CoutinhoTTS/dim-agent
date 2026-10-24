# 006：插件契约是概念与约定，而不是类型

[English](../../adr/006-plugin-contract.md)

决策状态：已由仓库所有者接受（2026-10-24）。实现：尚未开始；各阶段见
[插件契约设计](../design/plugin-contract.md#10-迁移)。

与其他决策的关系：

- 扩展 [ADR 002](002-plugin-scope.md)。其 D1–D4 不变。本 ADR 补全 D5：
  Dim 自己的运行时服务成为契约中的命名条目，Rust 成为契约的一种绑定，而不是契约的定义。
- 保留 [ADR 001](../../adr/001-harness-foundation.md#events-and-ownership-issue-2)（信封、保留
  领域、描述性的事件声明）和 [ADR 003](003-approval-scope.md)（受信任的进程内插件不靠声明隔离）。
- 写在两份分开的设计里：[Dim 如何使用 rutis](../design/rutis-usage.md)（D2–D4）与[插件契约](../design/plugin-contract.md)（D5–D8）。
- 取代 [rutis 0.15 升级设计](../design/rutis-0.15-upgrade.md)的事件作用域模型
  （§4、D2–D4）。由 [rutis 0.16 升级设计](../design/rutis-0.16-upgrade.md)取而代之。
- 取代按层划分类型的编写 API 作为目标：`Capability` trait 及其 `keys`、
  `SessionScope`/`BranchScope`、`InSession`/`InBranch`/`SessionGate`/`BranchGate`、
  `SessionService`/`BranchService`，以及 `AgentBuilder::session_plugin`/`timeline_plugin`。
  在迁移替换它之前，这套 API 仍是已实现的 Rust 绑定。
- 替代未合并的实例挂载草案（#377）以及 #370 的决策。

## 背景

Dim 的架构建立在一套小理论之上。[时间总线](../../architecture/rust-agent/time-bus.md)
按序号确定顺序，让每份状态只有一个所有者，把变更作为指令交给所有者，让交接显式化，把 `before`
限定为变换草稿，从历史重建状态，且从不重放副作用。ADR 002 D1 加了一条位置规则：部件放在其状态
所有者所在的位置。这些都不依赖某种编程语言。

插件 API 及其相关指引用 Rust 类型表达这套理论：

| 概念 | 当前表达 | 在 `main`（dd87f6af）上的规模 |
| --- | --- | --- |
| 部件放在哪里 | `session_plugin(\|scope: &SessionScope\| ..)`、`timeline_plugin(\|scope: &BranchScope\| ..)`、`Capability::keys` | 199 个 `Capability` 实现 |
| 依赖来自哪个实例 | `InSession<T>`、`InBranch<T>`、`SessionGate<T>`、`BranchGate<T>`、`SessionService`/`BranchService` 标记 | 221 处使用 |
| 事件流的隔离 | `EventKey::of().instance(id)` | 9 处，rutis 0.13 已移除的 API |
| 给作者的规则 | AGENTS.md 第 1 条、[原则 §1](../../architecture/rust-agent/principles.md)、[single-tree](../../architecture/rust-agent/single-tree.md#实例键与可见性)、插件页面 README | 27 篇英文文档提到这些类型 |

由此产生四个后果。

1. **非 Rust 部件需要按层的绑定，重新推导 Rust 类型已编码的内容。**
   - App Server 根据 TypeScript 行的 `inject` 是否点名了分支服务来推断该行的组
     （`crates/dim-app-server/src/session_typescript.rs:373`），并硬编码每个组可以提供哪些服务族
     （`:400`、`:457-492`）。
   - 一个服务的组被声明了三次，彼此没有交叉校验：`SessionService`/`BranchService`
     标记、`build.rs` 中的 `service_in(.., "branch")`，以及 `BRANCH_SERVICES`。
   - 一个概念有两个标识。原生的工具贡献是 `TypeKey::keyed_dynamic::<ToolContribution>`，
     外来的是 `host_key_in("toolContribution:..")`。ToolRegistry 两种都读，并通过一个按同一字符串
     找到的旁挂取消令牌来发现正在退役的外来代次（`tool_registry.rs:486-509`）。
   - 宿主预先注册提供者名称（`session_tool`、`session_methods`、`session_prompt_source`、
     `session_tool_source`、`host_tools`），并把外来行的 `provides` 翻译成这些调用。
     ADR 002 D5 规定注册由插件声明，而不是由宿主侧的 `AgentBuilder` 调用完成。
   - Session 与 Settings 的通知只能通过 `Process::emit` 上的中继到达外来行
     （`:166`、`:251`、`:515`），而这是 rutis 0.13 已移除的 API。
   - TypeScript Session 行没有发布（`activated`）服务，而 Rust Session 部件有
     （[作者指南](../../typescript-plugin-authoring.md#plugin-shape-and-lifetime)）。
2. **每个消费方都在每种语言中重新实现同样的基础设施协议。** 每个有序消费方都要订阅、固定已提交的
   截点、折叠本地历史并跳过已覆盖的事件：`crates/dim-agent-capabilities/src/wait.rs:361-380`、
   `runtime/plugins/goal/branch.ts:120-128` 和作者指南的示例都这样做。一个 TypeScript 部件
   自己把原生错误变体翻译成 JSON-RPC `appError` 错误码（`runtime/plugins/goal/session.ts:15`），
   这是在插件内部做的边界转换。
3. **指引混杂了四类陈述**：理论、约定、当前的 Rust 写法，以及交付状态或历史。贡献者分不清哪句
   是规则、哪句在描述今天的代码。因此设计文档的“规则检查”小节只是复述这份清单，而不是回答它。
4. **rutis 在底层变了，从 0.13 到 0.16：**
   - 每个根一条平的事件总线。一个名字的每个监听器都会收到该事件，无论它在哪里注册。监听器看不到
     发送方的实例，事件载荷必须可 serde。
   - 按名称提供的实例服务。目录中的一个名字属于一个实例化组，并对每个行副本解析到包围它的、
     最近的该组实例。放在该组之外的行会在组合时失败，无论它是 Rust 还是其他任何运行时
     （`ServiceCatalog::register_instance`、`register_shared_instance`、`key_in`、`Build::instance`）。
   - `#[rutis::service]` 在提供 Rust 服务的实例中按名称提供该服务。
   - rutis 写明的设计：“依赖声明只包含服务名”。模型只在 Rust 内核中实现一次，其他语言是带一个
     小型 SDK 的叶子。rutis 自己的 agent 插件使用普通插件、领域服务 trait 和命名事件，没有自己的
     插件 trait。

0.15 升级设计（#361）围绕按作用域投递来规划，而 rutis 0.15.1 撤回了按作用域投递。#377 草案向
rutis 模型靠拢，但保留了本 ADR 拒绝的两样东西。它在名字之外保留了 Rust 的 `Service<T>` 标识。
它还提出由 loader 强制的 Capability 角色，这是一种访问检查，而 [ADR 003](003-approval-scope.md)
指出受信任的进程内插件没有这种检查。

## 决策

### D1. 分层，每层一个权威来源

| 层 | 内容 | 权威来源 | 变更途径 |
| --- | --- | --- | --- |
| Dim 架构：理论 | 序号、所有权、指令、交接、`before`、重建、副作用、按所有者放置 | [时间总线](../../architecture/rust-agent/time-bus.md)、[原则](../../architecture/rust-agent/principles.md)、[ADR 002](002-plugin-scope.md) | 决策记录 |
| Dim 架构：插件契约 | 实例树的含义、位置、Dim 的服务与公开通知、Timeline 事实及其消费、通知如何消费、部件关系、发布与关闭 | [插件契约](../design/plugin-contract.md) | 本 ADR 之下的设计变更 |
| 使用 rutis | 部件即 loader 行、按名字与组的服务、服务目录机制、跨语言、rutis 生命周期与事件总线 | [Dim 如何使用 rutis](../design/rutis-usage.md) | 本 ADR 之下的设计变更 |
| 绑定 | 一种语言如何写出两者：Rust trait 与宏、TypeScript/Python `definePlugin`、生成的声明 | 《Dim 如何使用 rutis》的附录、SDK 源码、生成的文件 | 实现变更 |

中间两层分开写。D2–D4 决定 Dim 如何使用 rutis；D5–D8 决定建立在其上的 Dim 架构。Dim 的规则只依据
rutis 保证的东西来写，从不依据 rutis 的 API；关于使用 rutis 的规则也不决定任何业务问题。

绑定不增加语义。当某个绑定需要一条任何一层都没有写明的规则时，先改那一层。Rust 是一种绑定：它是
最完整的一种，而不是定义。面向贡献者的规则（AGENTS.md、原则、模板）写在各自的层，具体写法链接到
某个绑定。

### D2. rutis 拥有插件机制；Dim 在其上定义自己的领域

rutis 提供以下内容，Dim 直接使用：

- 插件生命周期（apply、effects、unload、取消、截止时间）；
- 依赖门控、Pending 与重载传播；
- 按类型和按名称的服务、实例化组，以及按实例的名称解析；
- loader 行与配置；
- 语言运行时、跨语言调用、事件总线与诊断。

Dim 定义：

- 实例树，以及其中的实例何时打开和关闭（`agent`、`session` 和 `branch` 组）；
- Timeline 机制（追加、序号、批次、发布、`before`、Dispatcher）；
- 其命名服务与公开通知的目录；
- 事件契约与保留领域；
- 部件约定（D6）、装配 profile 与审计（D9）。

Dim 不增加以下任何一项：插件 trait、面向作者的作用域类型、解析器、清单（manifest）、角色字段、
第二个服务目录、第二条总线，或按语言划分的中继与适配器。如果某个需求看起来需要其中之一，先查
rutis。真正的缺口在 Dim 自己的机制内解决，遵循[原则 §4 第 8 条](../../architecture/rust-agent/principles.md)；
rutis 仍是外部依赖。

### D3. 部件是 loader 行；它的组就是它的位置

每个部件都是挂载为 loader 行的 rutis 插件：无论原生还是外来，内置还是可选。行的组说明它放在
哪里：进程（根）、`session` 或 `branch`。`agent` 组承载宿主的门面，而不是功能部件。位置照旧
遵循 ADR 002 D1。

位置不由任何其他东西编码：没有插件 trait，没有工厂签名，也不从注入的名字推断。需要实例标识的
Rust 工厂从 rutis（`Build::instance`、`Ctx::instance`）或 Dim 的标识服务读取它。

宿主的装配 profile 决定 Session 或分支在发布前要等待哪些行
（[single-tree](../../architecture/rust-agent/single-tree.md#结构)）。插件不声明自己是必需的。细节见
[Dim 如何使用 rutis §2](../design/rutis-usage.md#2-部件是-loader-行)。

### D4. 服务是某个组中的一个名字，目录就是契约

- **标识。** 一个部件提供给另一个部件的每个服务都有一个 camelCase 名字，且恰好属于一个组：
  进程、`session` 或 `branch`。一族服务加一个限定符，如 `toolContribution:<tool>`。名字从不
  回退到另一个组（rutis 规则）。
- **一个目录。** 每个 Dim 名字只声明一次。跨语言的服务在其 Rust 类型上用 `#[rutis::service]`
  声明名字；仅限 Rust 的服务，或由外来部件提供的名字，在宿主的 loader 目录中注册。目录只共享
  已注册且已标记的名字（`share_registered_only`），因此跨语言的恰好就是这份目录。`xtask`
  从真实装配把它输出为一张生成的表：名字、组、提供者、方法（同步或异步，serde 请求与回复形状）、
  错误，以及该服务是跨语言还是仅限 Rust。
- **在每种语言中都按名称依赖。** rutis 沿每个副本的实例链为它解析每个名字，并拒绝点名了自身不在
  其内的组的行。Rust 部件使用相同的名字：Rust 绑定根据目录和副本的实例链推导 key。
  `TypeKey::of::<T>()` 仍是名字背后的 key，而不是第二个标识。
- **仅限 Rust 的服务仍在目录中。** 方法无法跨语言的服务（在提交锁内执行的回调、句柄、通道）
  仍然有名字和组，目录将其标记为仅限 Rust。要让它跨语言，就给它 serde 形状的方法；不存在手写的
  适配层。
- **错误是数据**，`{code, message}`，错误码有文档说明（ADR 002 D5）。服务错误由 wire 边界
  （Rpc）映射为 wire 错误，而不是由转发它的部件映射。

- **一族服务的提供者按名称查找。** 一族服务的消费方（`toolContribution:*` 的 ToolRegistry、
  `sessionMethods:*` 的 Rpc、提示词来源的 PromptRegistry）按名称查找在其实例中已激活的提供者，
  不论其语言。宿主不预先注册提供者名称，每个提供者只有一个标识，而不是一个原生标识加一个外来
  标识。

实现将移除：`Capability` trait 及其 key、`SessionScope`/`BranchScope`、
`InSession`/`InBranch`/`SessionGate`/`BranchGate`、`SessionService`/`BranchService`、
`session_plugin`/`timeline_plugin`、宿主侧的名称预注册、族提供者的原生/外来之分、App Server
对行所属组的推断，以及目录中的组已经表达的按族位置检查。`PublicService` 保留。它
限制 Session 与分支门面向普通调用方暴露哪些分支服务，这是门面规则，不是插件规则。

细节见 [Dim 如何使用 rutis §3](../design/rutis-usage.md#3-按名字的服务)；Dim 定义的名称列在
[插件契约 §4](../design/plugin-contract.md#4-dim-的服务)。

### D5. Timeline 事实与总线通知是两回事

| | Timeline 事实 | 总线通知 |
| --- | --- | --- |
| 是什么 | 一个分支的持久、有序号的事实，采用 `AgentEvent` 信封 | 根的平总线上一个短暂的 rutis 事件 |
| 写入方 | 分支的 `timeline` 服务；由生产者保证载荷 | 任何部件，通过 rutis `emit`/`parallel` |
| 投递 | 分支的 `dispatcher`：按序号顺序，本代次注册的每个部件都收到每一条事实 | 只投给发出时已注册的监听器；不存储、不回放；不同名字之间没有顺序；`emit` 不等待 |
| 消费方如何恢复 | 从 Timeline 重建 | 按[时间总线 §3](../../architecture/rust-agent/time-bus.md) 对每个状态机的要求：从 Timeline 恢复，或重新读取所有者的状态。漏掉一条通知只会推迟决定，不会改变它 |
| 标识 | 领域与名字；核心领域是保留的 | 目录中的公开名字；serde 载荷 |
| 隔离 | 按分支，由构造保证 | 按名字。属于某个实例的通知在名字中带上该实例的标识，由所有者导出的构造函数生成；或者由所有者改为提供 watch 或服务方法 |

实例界定生命周期与服务可见性。它们不界定事件投递。Dispatcher 与 Timeline 内部使用的总线名字
归它们所有。部件通过 `dispatcher` 消费，通过 `timeline.before` 注册变换，从不使用那些名字。
rutis 保留 `rutis/*` 和每个运行时的 `internal/*`。Dim 为其私有的机制通道保留 `dim/*`，任何
部件都不在其上发送或监听。

### D6. 角色是约定，由部件与 Timeline 的关系定义

| 关系 | 部件在以下情况具有该关系 | 用到 |
| --- | --- | --- |
| **Capability** | 持有按序号顺序从分支事实折叠出的状态，或写入自己领域的事实 | `dispatcher`、`timeline` |
| **提交方（Submitter）** | 通过所有者的入口向分支写入输入或指令，且不持有从该分支派生的状态 | `treeCommands.input`、Session 输入、指令事件 |
| **提供方（Provider）** | 向其他部件提供命名服务：工具、方法、提示词来源、宿主端口 | 服务目录 |

一个部件可以同时具有几种关系。关系本身不增加规则：规则就是现有的那些（时间总线、原则 §4、
ADR 002），关系只说明其中哪些通常适用（[契约 §6](../design/plugin-contract.md#6-部件关系)）。Timeline、Dispatcher、TimelineTree、作用域根与门面是各有
契约的机制，不是这个意义上的部件。

这改变了这个词的含义。此前，原则 §1 把每个 Session 或分支部件都叫作 Capability。现在
Capability 是分支 Timeline 上的状态机。只提供工具或方法、或只提交输入的 Session 部件不是
Capability。能力重载协议（[single-tree](../../architecture/rust-agent/single-tree.md#能力重载协议)）
适用于 Capability。

关系不声明，也不强制：没有角色字段，没有 trait，也没有 loader 检查。受信任的进程内部件不靠声明
隔离（ADR 003、AGENTS 第 11 条），而在平总线上，loader 检查能覆盖注入，却覆盖不了监听。这样的
检查看起来像边界，实际上不是。部件的关系体现在它注入和提供的名字中；审计（D9）从真实装配推导出
这些关系。

### D7. 机制保证什么，不保证什么

机制保证：

| 方面 | 保证 | 所有者 |
| --- | --- | --- |
| 可见性 | 一个组的名字只在该组的实例内解析；位置错误的行在组合时失败 | rutis |
| 暴露 | 只有目录中的名字跨语言 | rutis 目录、Dim 注册 |
| Timeline 完整性 | 信封检查，保留的核心领域按其类型化契约解码，原子批次，序号，提交后发布，关闭后拒绝 | Timeline（ADR 001） |
| 顺序 | 按分支、按序号顺序投递；重建截点的语义 | Dispatcher |
| 生命周期 | apply、effects、unload、取消与截止时间；只有在 profile 要求的行都 Active 之后才发布 | rutis，Session 与分支装配 |
| `before` | 进程内、同步、在提交锁内。任何外来运行时都不注册它（[ADR 001](../../adr/001-harness-foundation.md#trust-and-cross-language-calls)：提交关键区内没有跨进程代码） | Timeline |

其余的一切，即时间总线、原则 §4 和 ADR 002 的规则，都是约定，由评审、审计（D9）和流程测试检查。

### D8. 每个消费方都要重复的协议归机制所有者

- **带重建的有序消费成为一个 Dispatcher 操作。** 一次注册先投递直到一个固定截点的本地历史，
  再投递截点之后的实时事件，在每种语言中都如此。手写形式仍然有效。
- **Session 组和分支组各有一个生命周期服务，面向每种语言：** `session` 中的 `sessionLifecycle`
  （`activated`、`closing`、`hold`）和 `branch` 中的 `branchLifecycle`（`activated`、`closing`）。进程组没有；
  进程部件经 rutis 生命周期跟随宿主的启动与关闭。今天 `sessionLifecycle` 仅限 Rust，由 R2 暴露。
- **Rpc 把错误映射到 wire。** 贡献的方法的错误在 Rpc 处映射。部件重新抛出它收到的原生错误数据。

这些以需求 R1–R3 的形式写在契约的[需求一节](../design/plugin-contract.md#9-向机制所有者提出的需求)，
采用原则 §4 第 8 条的格式；由 Dim 运行时与 App Server 负责。SDK 可以在契约之上加语法糖，但不能
加语义（D1）。

### D9. 指引分层，审计自动生成

- 每条规则只在一处陈述。AGENTS.md 与原则 §1 陈述规则时不带 Rust 写法，Rust 写法放在 Rust 绑定中。
  契约链接到规则，而不复述规则。
- 设计文档的“规则检查”写明每个部件具有的关系，并照旧回答原有清单：原则 §4 第 1–9 条、§1 和
  ADR 002 D1–D5。拉取请求模板不变。
- 交付状态与历史放在 ADR 的交付表与设计文档中，不放在规则中。
- `xtask` 从真实装配（部件注入和提供的名字）推导每个部件的关系，并写入生成的文档。它检查每个
  插件页面的规则检查都写明了它的关系。

### D10. 工作顺序

1. rutis 0.16 升级，按[其自身的设计](../design/rutis-0.16-upgrade.md)进行。中继随之移除：各行直接监听公开通知。
2. 目录与 Rust 名称绑定。
3. 每个部件迁移为按名称依赖的行，族提供者按名称查找，删除层类型与宿主侧的预注册。旧类型只存在到这一步
   迁完所有部件为止。
4. 跨语言对齐：外来行声明自己的组，Session 组和分支组在每种语言中获得自己的生命周期服务，Rust 服务通过
   目录导出。
5. D8 中的机制需求。
6. 指引与生成的文档随每一步更新。

Dim 尚未发布，所以任何一步都不保留兼容路径：不为仓库外的任何人留别名、垫片或过渡形态。TypeScript SDK、
仓库内的插件、私有的托管 provider 插件和文档，随需要它的那一步一起修改。本设计不改变任何持久化格式或 wire 格式。

## 后果

- 一个服务在各语言之间只有一个标识，因此同一实例中的 Rust 与 TypeScript 部件按名称使用彼此的
  服务。
- Dim 特有的类型更少。插件页面与设计文档不再复述 Rust 写法。
- Capability 一词的含义收窄。把它用于每个 Session 部件的文档在下次改动时修订；原则 §1 现在修订。
- 角色不强制，因此有缺陷的受信任部件仍可能写入另一个所有者的领域，或在任何地方监听。和现在一样，
  由评审、审计与流程测试防范这一点。
- 私有的托管 provider 插件跟进每一次 SDK 变更。
- `docs/architecture/rust-agent/plugins/` 下的 33 个插件页面在第 3 步随其部件一起迁移。

## 考虑过的替代方案

| 替代方案 | 不采用的原因 |
| --- | --- |
| 保留层类型并加 Rust 语法糖（`Service<T>`，#370/#371） | 在名字之外增加第二个标识；外来部件仍需宿主侧推断位置 |
| 由 loader 强制的 Capability 角色（#377） | 看起来像隔离，实际上不是（平总线、受信任插件）；还要维护一份特权服务清单 |
| 保留按层的跨语言绑定（C4、C5、进程） | 每个都要重新推导位置、生命周期与投递，而且它们已经出现分歧（C4 中没有 Session 发布） |
| 把约定放进 SDK 辅助函数 | 在每种语言中复制一遍框架，违背 rutis 的设计与 D1 |
| 按作用域的事件投递（0.15 设计） | 已被 rutis 0.15.1 撤回 |
