# Dim 如何使用 rutis

[English](../../design/rutis-usage.md)

状态：已接受，见 [ADR 006](../adr/006-plugin-contract.md)（D2–D4）。尚未整体实现；各节说明今天已成立的部分。

插件机制归 rutis。本文说明 Dim 怎样使用它：部件如何装载，如何按名字提供和查找服务，服务如何跨语言，
以及 rutis 的生命周期和事件总线提供什么。它适用于每个部件，不论用哪种语言编写；附录是 Rust 与
TypeScript 的绑定。

本文不讲 Dim 的架构。实例的含义、Timeline 事实、所有权、部件关系、发布与关闭，见[插件契约](plugin-contract.md)，
它建立在[时间总线](../../architecture/rust-agent/time-bus.md)与[设计规则](../../architecture/rust-agent/principles.md)之上。
本文的规则不决定任何业务问题，插件契约的规则也不依赖任何 rutis API。

## 1. Dim 从 rutis 取用什么

以下各项 Dim 直接使用，不做包装（[ADR 006 D2](../adr/006-plugin-contract.md#d2-rutis-拥有插件机制dim-在其上定义自己的领域)）：

- 插件生命周期：apply、effect、卸载、取消、期限；
- 依赖门控、Pending 与重载传播；
- 按类型和按名字的服务、实例化组，以及按实例解析名字；
- loader 行与配置；
- 语言运行时、跨语言调用、事件总线与诊断。

Dim 不增加插件 trait、面向作者的作用域类型、解析器、manifest、role 字段、第二套服务目录、第二条总线，
也不增加按语言的中继或适配器。

## 2. 部件是 loader 行

一个部件是一个 rutis 插件，恰好作为一行 loader 配置装载。行所在的组就是部件的位置。没有别的东西表达位置：
不是 trait，不是工厂签名，也不是从注入的名称推断。部件应放在哪个组，由 Dim 决定（[插件契约 §3](plugin-contract.md#3-位置)）。

| 字段 | 含义 |
| --- | --- |
| `id` | 配置中稳定的行标识 |
| `name` | 模块：内置名称、路径，或运行时前缀（`py:`、`bun:` …） |
| `group` | 进程、`session` 或 `branch` |
| `config` | 结构性配置，由插件的 schema 校验 |
| `inject` | 部件依赖的名称。外来插件自身的 `inject` 必须等于这个列表，或是它的子集 |
| `isolate`、`disabled` | rutis 隔离标签；该行是否关闭 |

- 实例化组中的行，在该组的每个实例中各装载一次。
- 工厂或模块顶层只读取自己的配置和标识，不做 I/O，也不启动工作；资源在 `apply` 中取得（§4）。
- 行只保存结构性配置。业务配置放在所有者的来源中（[ADR 002 D4](../adr/002-plugin-scope.md#d4-结构性配置与业务配置分离)）。

*现状：*内置 Rust 部件通过 `AgentBuilder::session_plugin`/`timeline_plugin` 以作用域类型注册。
App Server 从 TypeScript 行的 `inject` 推断它的组（R5）。

## 3. 按名字的服务

### 3.1 标识

- 服务是恰好属于一个组的 camelCase 名称。一族服务使用 `family:qualifier` 形式的名称，例如
  `toolContribution:<tool>`、`sessionMethods:<namespace>` 和 `managementMethods:<namespace>`。
- 对行的每个副本，名称解析到该名称所属组的最近外层实例。它从不回退到另一个组。一行若指向它
  不在其内的组，在组合时失败。
- 每个实例中每个名称只有一个提供者。同一实例中同名的第二个提供者会失败。
- 各语言的部件都依赖名称。Rust 类型 key 是 Rust 绑定存储名称的方式，不是第二个标识。
- 一族服务的消费方按名称找到其实例中已激活的提供者，不论它们用什么语言。宿主不预先注册提供者名称。
  Rust 消费方可以通过类型化绑定调用原生提供者，但它找到原生和外来提供者的方式相同（R6）。

### 3.2 形状

| 要素 | 形式 |
| --- | --- |
| 方法 | 名称，`sync` 或 `async`。其他语言的调用方总是得到 Promise |
| 请求与回复 | serde 数据，以字段表或 JSON 结构描述 |
| 错误 | 数据，`{code, message}`，错误码由服务写明。转发的部件重新抛出它收到的错误，由 wire 边界映射（[插件契约](plugin-contract.md#9-向机制所有者提出的需求) R3） |
| 取消 | 一个协议动作：请求中的取消回调或 promise，或消费方丢弃该调用 |
| 回调 | 按引用，在 rutis 允许的范围内。回调不得在提交锁内被调用 |
| 流 | 回调或通知；rutis 没有流类型 |

### 3.3 跨语言

一个服务提供的每个方法都有 serde 参数和结果时，它就能跨语言。不能跨语言的方法，例如接受借用值、
通道、句柄或提交锁内同步回调的方法，仅限 Rust。该服务仍有名称和组，服务目录把该方法或整个服务
标为仅限 Rust。要让它跨语言，就改变它的形状。不存在手写的适配层。

### 3.4 服务目录

每个名称只声明一次。跨语言的服务在其 Rust 类型上用 `#[rutis::service]` 声明名称。仅限 Rust 的服务，
或由外来部件提供的名称，注册在宿主的 loader 服务目录中。服务目录只共享这些名称（`share_registered_only`）。
`xtask` 将从真实装配生成权威的表（R4）。Dim 定义的名称列在[插件契约 §4](plugin-contract.md#4-dim-的服务)。

## 4. rutis 提供的生命周期

| 阶段 | 部件可以 | rutis 做什么 |
| --- | --- | --- |
| 构建 | 读取自己的配置和标识 | 构造插件；不做 I/O |
| `apply` | 取得资源、提供服务、注册订阅、登记清理 | 按名称解析依赖并以它们门控，每个代次运行一次 `apply` |
| 卸载 | — | 按相反顺序运行清理，把部件的监听器从总线移除，取消其正在运行的调用，并最多等待到 rutis 的期限 |

一个代次是部件从 `apply` 到卸载的一次装载。某个依赖被替换时，rutis 沿声明的依赖卸载并重新装载其下游。
Dim 在此之上增加发布与关闭两个阶段（[插件契约 §7](plugin-contract.md#7-部件的生命周期)）。

## 5. 事件总线

从 rutis 0.15.1 起，每个根只有一条平总线。

- **投递。** 某个名称的每个监听器都收到该名称的每个事件，无论监听器在哪里注册。监听器看不到发送方的实例。
  事件只到达发出时已经注册的监听器，不存储、不回放。不同名称的事件之间没有共同顺序。`emit` 不等待；
  `parallel` 等待并汇总失败。
- **载荷是 serde。** 事件中不放句柄、通道或锁。
- **名称就是契约。** 属于某个实例的事件，在名称中带上该实例的标识，名称由其所有者导出的构造函数生成，
  因此调用方从不拼接名称。或者所有者改为提供 watch 或服务方法。
- **保留前缀：**`rutis/*`（rutis 的框架事件：只可监听）和 `internal/*`（各运行时自有）。

Dim 的部件如何消费通知、Dim 自己保留的前缀，以及 Dim 的公开通知，见[插件契约 §5.3](plugin-contract.md#53-总线通知)。

*现状：*Dim 仍在 rutis 0.12.1 上，事件带实例键；[0.16 升级](rutis-0.16-upgrade.md)把它移到平总线上。

## 6. 向机制所有者提出的需求

这些需求关于 Dim 如何使用 rutis。它们遵循[原则 §4 第 8 条](../../architecture/rust-agent/principles.md)，
所属阶段见[插件契约的迁移](plugin-contract.md#10-迁移)。R1–R3 在[插件契约](plugin-contract.md#9-向机制所有者提出的需求)中。

### R4. 服务目录自动生成，关系审计自动化

- **受阻：**本文 §3.4 与[插件契约 §8](plugin-contract.md#8-保证约定与审计)。今天服务目录分散在 `build.rs`、作者指南和设计文档中，关系也没有审计。
- **考虑过的现有机制：**`xtask graph` 已经从真实装配记录注入和提供的服务。
- **所需能力：**`xtask graph` 写出服务目录表（名称、组、提供者、是否跨语言、方法）和每个部件的关系。
  `xtask check` 验证每个插件页面的规则检查写明了它的关系，并标出机制之外对 `dim/*` 的使用。
- **影响：**运行时没有影响。
- **所有者：**`xtask`。issue 待开。

### R5. 外来行声明自己的组

- **受阻：**§2。App Server 从 TypeScript 行的 `inject` 推断它的组，并按组检查服务族
  （`session_typescript.rs:373`、`:400`）。
- **考虑过的现有机制：**rutis 行已经有 `group`，服务目录在组合时拒绝放错位置的名称。
- **所需能力：**外来行像原生行一样携带自己的组。推断和族检查被删除。（通知中继更早在
  [0.16 升级](rutis-0.16-upgrade.md) U5 中去掉。）
- **影响：**对 Timeline 没有影响。
- **所有者：**App Server。issue 待开。它在阶段 4 落地，SDK 与仓库内的插件一起改。

### R6. 族的提供者按名称查找，只有一个标识

- **受阻：**[ADR 002 D5](../adr/002-plugin-scope.md#d5-合同语言中立)
  （“注册：由插件声明……而非宿主侧的 `AgentBuilder` 调用”）。宿主预先注册提供者名称
  （`session_tool`、`session_methods`、`session_prompt_source`、`session_tool_source`、`host_tools`）。
  原生工具贡献和外来工具贡献有不同的 key，因此 ToolRegistry 两者都读，并通过以同一字符串找到的
  sidecar 令牌检测正在退役的外来代次（`tool_registry.rs:486-509`）。提示词来源和工具来源不对外来部件开放。
- **考虑过的现有机制：**ToolRegistry 已经在每次读取时接入当时已激活的提供者，并撤下正在退役的提供者
  （ADR 002 D3，#241）。rutis 用 `rutis/service` 事件报告提供者的出现和离开，标记过的 Rust 服务
  也在其实例中按名称提供。
- **所需能力：**每个族的消费方（ToolRegistry、PromptRegistry、Rpc 的方法表）按名称找到它自己实例中
  已激活的本族提供者。原生和外来提供者只有一个标识和一条发现路径；它们的服务被撤下时，它们的代次结束。
  预先注册和 sidecar 令牌被去掉。
- **影响：**对 Timeline 没有影响。工具采样保持 ToolBinding 现有的逐 turn 规则。
- **所有者：**Dim 运行时（ToolRegistry、PromptRegistry）和 App Server（Rpc）。issue 待开。
  它在阶段 3 落地。

## 附录 A. Rust 绑定

### A.1 现状与目标

| 现状 | 目标 |
| --- | --- |
| `impl Capability for P { fn keys(&self) }` | 没有 Dim trait。部件是 rutis `Plugin` 或 `TypedPlugin` |
| 传给工厂的 `SessionScope`、`BranchScope` | 来自 rutis 的副本实例链（`Build::instance`、`Ctx::instance`）；标识来自 `sessionIdentity` 或 `timelineIdentity` |
| `InSession<T>`、`InBranch<T>` | 对名称的依赖；key 由服务目录为这个副本给出 |
| `SessionGate<T>`、`BranchGate<T>` | 对名称的门控（使用服务目录 key 的 rutis `KeyedGate`） |
| `Contributions<T>` | 在实例中找到的一族名称（R6） |
| `When<D>` | 由行的配置启用的依赖：工厂构建副本时决定是否包含该名称。不是 rutis `Option<..>`，后者读取但不门控 |
| `SessionService`、`BranchService` 标记 | 注册在服务目录中的组 |
| `AgentBuilder::session_plugin`/`timeline_plugin` | 装配 profile 中 `session` 或 `branch` 组的行 |
| `AgentBuilder::session_tool`、`session_methods`、`session_prompt_source`、`session_tool_source`、`host_tools` | 无：提供者按名称找到（R6） |
| `build.rs` `Exports::service_in(..)` | 类型上的 `#[rutis::service]`；`Exports` 只用于源码无法修改的类型 |
| `EventKey::of().instance(id)` | 已被 rutis 0.13 移除；见 §5.3 和 0.16 升级 |
| `PublicService` | 不变（门面规则） |

### A.2 草图

确切的 API 在 P2 中决定。草图只表达意图：名称来自服务目录，部件中没有任何东西说明它在哪个组。

```rust
// Target sketch, not an API.
pub struct WaitBranch;
impl TypedPlugin for WaitBranch {
    // Keys resolved by the catalog for this copy: "dispatcher" and "timeline" are branch names.
    type Deps = (Keyed<Dispatcher>, Keyed<Timeline>);
    fn name(&self) -> &str { "WaitBranch" }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (dispatcher, timeline): Self::Deps)
        -> BoxFuture<'a, Result<Effect, CordisError>> { /* rebuild (plugin contract §7.2), then provide "wait" */ }
}

// Assembly: one row in the branch group; the factory reads keys from the catalog.
builtins.register_with("WaitBranch", |build| catalog.typed::<WaitBranch>(build));
```

## 附录 B. TypeScript 与 Python 绑定

部件使用 `definePlugin`（Python 中为 `define_plugin`），带 `inject` 和 `provides`。行携带组。
SDK 生成的声明来自服务目录。

```json
{ "id": "goal-branch", "name": "/abs/goal/branch.ts", "group": "branch",
  "inject": ["dispatcher", "timelineIdentity", "treeCommands", "settings", "branchLifecycle"],
  "config": {} }
```

```ts
// A Capability that rebuilds by hand (plugin contract §7.2, today's form).
export default definePlugin({
  name: 'ExampleCounter',
  inject: ['dispatcher'],
  provides: { exampleCounter: { get: 'sync' } },
  async apply(ctx) {
    const dispatcher = ctx.use<BranchServices['dispatcher']>('dispatcher')
    let count = 0, through = 0n
    let ready!: () => void
    const rebuilt = new Promise<void>(resolve => { ready = resolve })
    await dispatcher.listen('example-counter', ['input/message'], async event => {
      await rebuilt
      if (BigInt(event.sequence) > through) count++
    })
    const cutoff = await dispatcher.committed()
    count = (await dispatcher.local(0, cutoff))
      .filter(event => event.type === 'input' && event.name === 'message').length
    through = BigInt(cutoff)
    ctx.provide('exampleCounter', { get: () => ({ count }) })
    ready()
  },
})
```

有了 R1，同一个部件只注册一次，并在一个处理函数中折叠每个事件，无论是重放的还是实时的。

*现状：*行的组是推断出来的，不是声明的（R5）。Session 行没有生命周期服务（插件契约 R2）。通知经
`session.event` 中继到达，直到 0.16 升级把它去掉。
