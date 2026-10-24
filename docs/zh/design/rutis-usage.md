# Dim 如何使用 rutis

[English](../../design/rutis-usage.md)

状态：已接受，见 [ADR 006](../adr/006-plugin-contract.md)（D2–D4）。尚未整体实现；各节说明今天已成立的部分。

插件机制归 rutis。本文说明 Dim 怎样使用它：部件如何装载，如何按名字提供和查找服务，服务如何跨语言，
以及 rutis 的生命周期和事件总线提供什么。它适用于每个部件，不论用哪种语言编写；附录是 Rust 与
TypeScript 的绑定。

本文不讲 Dim 的架构。实例的含义、Timeline 事实、所有权、Capability、发布与关闭，见[插件契约](plugin-contract.md)，
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

## 2. 插件是一行；实例挂载它的部件

一个插件是一个 rutis 插件，恰好作为进程里的一行 loader 配置装载。它的 Session 部件和分支部件不是行：由插件登记，
由每个实例挂载（[ADR 006 D3](../adr/006-plugin-contract.md#d3-插件是一行每个实例挂载为它登记的部件)）。部件应放在
哪个作用域，由 Dim 决定（[插件契约 §3](plugin-contract.md#3-位置)）。

| 行字段 | 含义 |
| --- | --- |
| `id` | 配置中稳定的行标识 |
| `name` | 模块：内置名称、路径，或运行时前缀（`py:`、`bun:` …） |
| `config` | 结构性配置，由插件的 schema 校验 |
| `inject` | 插件依赖的名称。外来插件自身的 `inject` 必须等于这个列表，或是它的子集 |
| `isolate`、`disabled` | rutis 隔离标签；该行是否关闭 |

**登记部件。** 部件登记处是一个进程服务。插件在自己的 `apply` 里为 `session` 或 `branch` 登记生成器。登记是调用方的
一个 effect，和 Cordis 把 `ctx.on` 绑定到调用方一样：插件的这一代运行多久，登记就保持多久；这一代无论怎样结束，
登记都会撤销。生成器的依赖在登记时按它的作用域检查，所以点名了作用域之外的组的部件在登记时就失败。

**挂载。** 每个 Session 实例和分支实例里都有一个宿主部件，即它的挂载器。除了作用域自己的机制，它是该组唯一的一行。

- 实例打开时，挂载器调用为它的作用域登记的所有生成器，用 rutis 的 `ctx.plugin` 把每个结果挂成自己的子节点
  （对这个实例什么都不返回的生成器就不挂），
  并等装配 profile 要求的部件都进入 Active，实例这时才发布。profile 之外的部件失败时会被报告，只影响它自己。
- 实例打开期间，挂载器跟随登记处：新登记的就挂上，撤销的就卸下。所有实例都卸载完各自那一份，撤销才算结束。
- 实例关闭时，挂上的部件随它一起关闭，由 rutis 对子节点的所有权保证。

**生成器返回什么。**

- Rust 生成器拿到它所挂进的实例的名字（附录 A），返回一个插件。这个插件可以带着插件在 `apply` 里准备好的东西。
- 外来生成器返回模块名和配置。挂载器通过运行时解析器解析这个模块，为自己的实例加载一份副本，和实例化组里的行
  一样加载（用该实例的 `Build` 调用 `Resolved::scoped`）。副本自己声明 `inject` 和 `provides`。插件在 `apply` 里准备的
  东西，通过两者都导入的模块（同一个运行时进程加载二者）或通过服务交给副本。R7 让外来生成器也能是一个函数。

**每个插件和部件：**

- 工厂或模块顶层只读取自己的配置和标识，不做 I/O，也不启动工作；资源在 `apply` 中取得（§4）。
- 行只保存结构性配置。业务配置放在所有者的来源中（[ADR 002 D4](../adr/002-plugin-scope.md#d4-结构性配置与业务配置分离)）。
- 部件可以在自己的子树里创建子节点，比如 McpSession 为每个服务器各建一个。这些是部件自己的事。

*现状：*Session 部件和分支部件是 `session`、`branch` 组的行。按名称依赖的 Rust 部件通过 `AgentBuilder::session_part`/
`branch_part` 添加，进程部件通过 `register_part` 添加（附录 A），其余部件通过 `AgentBuilder::session_plugin`/
`timeline_plugin` 以作用域类型注册。其他语言的行在 App Server 的配置中携带自己的组（附录 B），每个部件一行。
P3 和 R5 用部件登记处取代这些。

## 3. 按名字的服务

### 3.1 标识

- 服务是恰好属于一个组的 camelCase 名称。成组出现的服务共用一个前缀，使用 `prefix:qualifier` 形式的名称，例如
  `toolContribution:<tool>`、`sessionMethods:<namespace>` 和 `managementMethods:<namespace>`。
- 对行的每个副本，名称解析到该名称所属组的最近外层实例。它从不回退到另一个组。一行若指向它
  不在其内的组，在组合时失败。
- 每个实例中每个名称只有一个提供者。同一实例中同名的第二个提供者会失败。
- 各语言的部件都依赖名称。Rust 类型 key 是 Rust 绑定存储名称的方式，不是第二个标识。
- 带前缀服务的消费方按名称找到其实例中已激活的提供者，不论它们用什么语言。宿主不预先注册提供者名称。
  Rust 消费方可以通过类型化绑定调用原生提供者，但它找到原生和外来提供者的方式相同（R6）。

### 3.2 形状

| 要素 | 形式 |
| --- | --- |
| 方法 | 名称，`sync` 或 `async`。其他语言的调用方总是得到 Promise |
| 请求与回复 | serde 数据，以字段表或 JSON 结构描述 |
| 错误 | 数据，`{code, message}`，错误码由服务写明。转发的部件重新抛出它收到的错误，由 wire 边界映射（[插件契约](plugin-contract.md#9-向机制所有者提出的需求) R3） |
| 取消 | 一个协议动作：请求中的取消回调或 promise，或消费方丢弃该调用 |
| 回调 | 按引用，在 rutis 允许的范围内。回调不得在提交锁内被调用 |
| 流 | 回调或事件；rutis 没有流类型 |

### 3.3 跨语言

一个服务提供的每个方法都有 serde 参数和结果时，它就能跨语言。不能跨语言的方法，例如接受借用值、
通道、句柄或提交锁内同步回调的方法，仅限 Rust。该服务仍有名称和组，服务目录把该方法或整个服务
标为仅限 Rust。要让它跨语言，就改变它的形状。不存在手写的适配层。

### 3.4 服务目录

每个名称只声明一次。跨语言的服务在其 Rust 类型上用 `#[rutis::service]` 声明名称。仅限 Rust 的服务，
或由外来部件提供的名称，注册在宿主的 loader 服务目录中。服务目录只共享这些名称（`share_registered_only`）。
`xtask` 将从真实装配生成权威的表（R4）。Dim 定义的名称列在[插件契约 §4](plugin-contract.md#4-dim-的服务)。

*现状：*跨语言的进程服务标记了 `#[rutis::service]`。按 Session 或分支提供的服务则连同它们的组列在
App Server 的构建脚本中（`Exports::service_in`）：rutis 0.16 中，标记过的类型在某个实例中提供时，会在那里
*认领*它的名字，而每次认领都运行一次覆盖整个 loader 的 reconcile，这也会重建那些构建被拒绝的副本。
于是，在某个 Session 的装配中失败的必需行可能在该 Session 创建期间被重建，而每次创建 Session 或分支都会
reconcile 整个 loader。两种形式都让每个类型只有一个名字，并由一个测试对照 Rust 绑定检查。

### 3.5 各运行时如何解析名字

§3.1 的规则归框架（[ADR 006 D4](../adr/006-plugin-contract.md#d4-服务是某个组中的一个名字目录就是契约)）：
loader 为每个副本解析每个名字，各运行时落实结果。部件代码只写名字。

| 运行时 | 部件写的 | 由什么解析出拿到哪一份 |
| --- | --- | --- |
| Node（TypeScript、Cordis 4 插件） | `inject: [name]`、`ctx.use(name)` 或 `ctx.<name>`、`ctx.provide(name, value)` | loader 解析名字（`key_in`），并在 Node 进程中把它隔离在该实例的标签下（Cordis 的 `isolate`） |
| Python 和其他运行时 | `inject`、`use`、`provide` | 同上，通过该运行时的隔离表 |
| Rust | `Keyed<T>`；在同一个 key 上提供 `T` | 绑定按名字和副本的实例链从目录取得 key（附录 A） |

- **私有名字。** 部件用自己行上的 `isolate` 把一个服务留给自己或一组部件。目录不共享这样的名字。
- **Rust 的 key。** 用 Rust 类型登记的名字位于该类型的 key；跨语言的名字位于它的分发器的 key（`host_key`）。
  两者都用组的实例限定。Rust 部件在分发器的 key 上读取任何语言的提供方。导出的 Rust 服务两者都有。
- **提供的名字。** 行没有声明、在运行时才宣告的服务，rutis 以其普通名字在整个进程内发布。在 Session 或分支内，
  只属于该实例的名字要以其组登记，这样每个实例各有一份；部件私有的名字由它的行隔离。否则其他实例也能看到该服务，
  第二个提供它的副本会因 rutis 的重复提供者错误而失败。
- **必需与可选。** 必需的依赖要注入，并对部件门控。可选的依赖在使用时读取，不注入：Cordis 的 `ctx.get(name)`、
  rutis 的 `Option<Keyed<T>>`。由配置选择的依赖，是否包含在副本的名字中由配置决定。
- **Cordis。** 未经修改的 rutis Cordis 4（`@deepseek-ai/cordis`）插件作为 Node 行运行，得到同样的解析。
  Cordis 3 插件不在目标内。

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
- **顺序。** 同一名称的事件按发出顺序启动监听器：上一个事件的每个监听器都已启动后，下一个事件的监听器
  才启动；没有谁等待一个正在 await 的监听器。需要顺序的监听器在第一次 await 之前做完有序的部分，
  或者通过 `dispatcher` 消费事实（[插件契约 §5.2](plugin-contract.md#52-消费事实)）。
- **载荷是 serde。** 事件中不放句柄、通道或锁。
- **名称就是契约。** 属于某个实例的事件，在名称中带上该实例的标识，名称由其所有者导出的构造函数生成，
  因此调用方从不拼接名称。或者所有者改为提供 watch 或服务方法。
- **保留前缀：**`rutis/*`（rutis 的框架事件：只可监听）和 `internal/*`（各运行时自有）。

Dim 的部件如何消费总线事件，以及 Dim 定义了哪些公开总线事件，见[插件契约 §5.3](plugin-contract.md#53-总线事件)。

*现状：*Dim 已在 rutis 0.16.0 上，本节成立。

## 6. 向机制所有者提出的需求

这些需求关于 Dim 如何使用 rutis。它们遵循[原则 §4 第 8 条](../../architecture/rust-agent/principles.md)，
所属阶段见[插件契约的迁移](plugin-contract.md#10-迁移)。R1–R3 在[插件契约](plugin-contract.md#9-向机制所有者提出的需求)中。

### R4. 服务目录自动生成，自动标出 Capability

- **受阻：**本文 §3.4 与[插件契约 §8](plugin-contract.md#8-保证约定与审计)。今天服务目录分散在 `build.rs`、作者指南和设计文档中，也没有东西标出哪些部件是 Capability。
- **考虑过的现有机制：**`xtask graph` 已经从真实装配记录注入和提供的服务。
- **所需机制：**`xtask graph` 写出服务目录表（名称、组、提供者、是否跨语言、方法），并根据部件注入的名字
  （`dispatcher`、`timeline`）标出哪些部件是 Capability。
- **影响：**运行时没有影响。
- **所有者：**`xtask`。issue 待开。

### R5. 外来插件登记自己的部件

- **受阻：**§2。一个有 Session 部件和分支部件的外来功能，每个部件都是一行配置，各自携带自己的组。R5 的最初形式
  （外来行声明自己的组，不再由 App Server 从 `inject` 推断）已在阶段 4 交付（[#376](https://github.com/arcships/dim/issues/376)），
  但插件仍然不是一行。
- **考虑过的现有机制：**运行时解析器已经能为一个模块所在的实例构造副本（`Resolved::scoped`），服务目录拒绝放错位置的名称。
- **所需机制：**外来插件是进程里的一行，向部件登记处登记自己的部件，每个部件是一个模块名和配置（§2）。其他语言的
  Session 行和分支行，以及它们声明的组，都删除。
- **影响：**对 Timeline 没有影响。
- **所有者：**App Server 和各 SDK，仓库内的插件一起改。它在 P3 的部件登记处之后进行。

### R6. 带前缀服务的提供者按名称查找，只有一个标识

- **受阻：**[ADR 002 D5](../adr/002-plugin-scope.md#d5-合同语言中立)
  （“注册：由插件声明……而非宿主侧的 `AgentBuilder` 调用”）。宿主预先注册提供者名称
  （`session_tool`、`session_methods`、`session_prompt_source`、`session_tool_source`、`host_tools`）。
  原生工具贡献和外来工具贡献有不同的 key，因此 ToolRegistry 两者都读，并通过以同一字符串找到的
  sidecar 令牌检测正在退役的外来代次（`tool_registry.rs:486-509`）。提示词来源和工具来源不对外来部件开放。
- **考虑过的现有机制：**ToolRegistry 已经在每次读取时接入当时已激活的提供者，并撤下正在退役的提供者
  （ADR 002 D3，#241）。rutis 用 `rutis/service` 事件报告提供者的出现和离开，标记过的 Rust 服务
  也在其实例中按名称提供。
- **所需机制：**每个带前缀服务的消费方（ToolRegistry、PromptRegistry、Rpc 的方法表）按名称找到它自己实例中
  已激活的、该前缀下的提供者。原生和外来提供者只有一个标识和一条发现路径；它们的服务被撤下时，它们的代次结束。
  预先注册和 sidecar 令牌被去掉。发现路径在每种语言中都一样：消费方监听 `rutis/service` 事件（名字、作用域、
  是否可用、行），保留其中前缀相符且作用域是自己实例的，再按名称读取每一个。
- **影响：**对 Timeline 没有影响。工具采样保持 ToolBinding 现有的逐 turn 规则。
- **所有者：**Dim 运行时（ToolRegistry、PromptRegistry）和 App Server（Rpc）。issue 待开。
  它在阶段 3 落地。

### R7. 每个运行时的生成器都能是一个函数

- **受阻：**§2。外来生成器返回模块名和配置，因为宿主只能按名字加载外来插件。按插件自身状态准备的部件，必须经过
  共享模块或服务，Rust 部件则不需要。
- **考虑过的现有机制：**模块名和配置覆盖了现有的每个部件，这是过渡形式。
- **所需机制：**运行中的行登记带声明的具名生成器，宿主能在指定实例里为"行 R 的生成器 G"构造工厂。每一份副本在
  生成者的运行时里运行，按实例的标签隔离，按生成器的 `inject` 门控；它属于挂载它的宿主 fiber；生成器撤销时结束。
- **影响：**对 Timeline 没有影响。
- **所有者：**rutis，[arcships/rutis#642](https://github.com/arcships/rutis/issues/642)。落地后 Dim 把各 SDK 切换过去；
  没有东西等它。

## 附录 A. Rust 绑定

### A.1 旧形式及其替代

替代形式自 P2 起可用；旧形式保留到 P3 迁移完所有部件为止。

| 旧形式 | 替代 |
| --- | --- |
| `impl Capability for P { fn keys(&self) }` | 没有 Dim trait。部件是 rutis `Plugin` 或 `TypedPlugin` |
| 传给工厂的 `SessionScope`、`BranchScope` | 工厂需要实例时用 `Names::instance(group)`；标识来自 `sessionIdentity` 或 `timelineIdentity` |
| `InSession<T>`、`InBranch<T>` | `Keyed<T>`：对 `T` 的名字的依赖，key 用这个副本实例链中该名字所属组的实例限定（§3.5） |
| `SessionGate<T>`、`BranchGate<T>` | 对该名字的 `KeyedGate<T>` |
| 提供服务时的 `scope.key::<T>()` | `names.key::<T>()`，与该名字的消费方拿到的 key 相同 |
| `Contributions<T>` | 在实例中找到的同一前缀下的名称（R6） |
| `When<D>` | `When<D>`，key 来自 `Names::when`：工厂构建副本时决定是否包含该名称。不是 rutis `Option<..>`，后者读取但不门控 |
| `SessionService`、`BranchService` 标记 | 注册在服务目录中的组（`names::register`） |
| `AgentBuilder::session_plugin`/`timeline_plugin` | `AgentBuilder::session_part`/`branch_part`：`session` 或 `branch` 组中的行；进程行用 `register_part` |
| `AgentBuilder::session_tool`、`session_methods`、`session_prompt_source`、`session_tool_source`、`host_tools` | 无：提供者按名称找到（R6） |
| `build.rs` `Exports::service_in(..)` | 类型上的 `#[rutis::service]`；`Exports` 用于源码无法修改的类型，以及按 Session 或分支提供的服务（§3.4） |
| `PublicService` | 不变（门面规则） |

### A.2 绑定

Rust 部件就是普通的 `TypedPlugin`。它的依赖是 `Keyed<T>`、`KeyedGate<T>`、`Option<Keyed<T>>`
（存在时读取，不门控）或它们的元组（`NamedDeps`）。绑定在 Rust 中落实 §3.5：

- 按 `#[rutis::service]` 的规则给类型命名：`Timeline` 是 `timeline`，`dyn LoopState` 是 `loopState`
  （`names::name_of`）。一个 Rust 类型就是一个名字，所以在两个组提供的类型要拆成两个类型，或归到一个组。
- key 按名字和副本的实例链从服务目录取得。`names.key::<T>()` 用于以 Rust 类型 `T` 登记或由它导出的名字：
  在该名字所属组的实例中是 `TypeKey::instance::<T>(id)`，进程级是 `TypeKey::of::<T>()`。
  `names.named::<T>(name)` 在目录的 key 上读取任意名字；对跨语言的名字，`T` 是 `dyn HostDispatch`，
  任何语言的提供方都在那里。提供方在它的消费方拿到的 key 上提供。
- 在组合装配时按行的位置检查名称。部件依赖的名称属于某个组而它的行不在该组内，就在此时失败，并指出该组；
  依赖未登记的服务也同样失败。

服务目录是 builder 的（`AgentBuilder::catalog`），App Server 的 loader 用的是同一个。
`names::register::<T>(catalog, group)` 声明一个 Rust 名称：运行时登记自己的名称，每个 crate 登记它提供的
服务的名称；同一名称在同一组再次登记不产生任何效果。导出的名称经生成的分发器提供给其他语言，一个测试
检查每个导出的名字和组与绑定按其类型推出的相同。

```rust
pub struct WaitBranch(DepKey<Wait>);
impl TypedPlugin for WaitBranch {
    // "dispatcher" and "timeline" are branch names; nothing here says so.
    type Deps = (Keyed<Dispatcher>, Keyed<Timeline>);
    fn name(&self) -> &str { "WaitBranch" }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (dispatcher, timeline): Self::Deps)
        -> BoxFuture<'a, Result<Effect, CordisError>> {
        /* rebuild (plugin contract §7.2), then ctx.provide_as(self.0.key().clone(), wait) */
    }
}

// A row in every branch; `names` gives this copy's keys, for its dependencies and for `wait`.
builder.branch_part(|names| names.part(WaitBranch(names.key::<Wait>()?)));
// A process row.
register_part(&mut builtins, builder.catalog(), |names| names.part(Recorder))?;
```

由配置启用的依赖在工厂中取得 key：
`names.part_with(plugin, (names.key::<Timeline>()?, names.when::<Keyed<FileStorage>>(enabled)?))`。

### A.3 已定的问题

来自 [#6](https://github.com/arcships/dim/issues/6)：

- **依赖消失。** 某个名称的提供者消失时，rutis 把依赖它的部件退回 Pending，提供者回来后再次加载。新的一代
  按能力重载协议从 Timeline 重新开始（[插件契约 §7.2](plugin-contract.md#72-代次重载与重建)），不再增加任何东西。
- **可选依赖与由配置选择的依赖。** 同 §3.5：由配置选择的依赖在被选中时门控（`When<D>`、`Names::when`），
  组合检查此时也覆盖它；只在存在时才用的依赖是 `Option<Keyed<T>>`，不门控。它的名称仍须登记。
- **启动期限。** 一个期限覆盖 loader 启动、reconcile 与各行变为 Active。到期时创建失败，回滚取消各行；
  rutis 等待被取消的 `apply` 返回，从不中止它，因此 `apply` 在 `ctx.cancelled()` 完成后返回。一个测试注入
  在 `apply` 中一直等到那时的行（`tests/integration/tests/named_parts.rs`）。改为“没有进展才判失败”见
  [#387](https://github.com/arcships/dim/issues/387)。

## 附录 B. TypeScript 与 Python 绑定

部件使用 `definePlugin`（Python 中为 `define_plugin`，或一个带 `inject`、`provides` 和 `apply` 的模块），
带 `inject` 和 `provides`。行携带组：`"session"`、`"branch"`，或者在进程中不带组。SDK 生成的声明来自服务目录。

```json
{ "id": "goal-branch", "name": "/abs/goal/branch.ts", "group": "branch",
  "inject": ["dispatcher", "timelineIdentity", "treeCommands", "settings", "branchLifecycle"],
  "config": {} }
```

Python 行的名字以运行时为前缀写出模块：`py:<module>`；配置了 Python 运行时时，App Server 在其中运行该行，
从配置的项目目录导入该模块。

```json
{ "id": "py-branch", "name": "py:python_branch_part", "group": "branch",
  "inject": ["dispatcher", "timelineIdentity", "treeCommands"], "provides": {}, "config": {} }
```

```python
inject = ["dispatcher", "timelineIdentity", "treeCommands"]
provides = {}

async def apply(ctx, config):
    dispatcher = ctx.use("dispatcher")
    identity = await ctx.use("timelineIdentity").get()
    ...
```

```ts
// A Capability that rebuilds by hand (plugin contract §7.2, today's form).
export default definePlugin({
  name: 'ExampleCounter',
  inject: ['dispatcher'],
  provides: { exampleCounter: { get: 'sync' } },
  async apply(ctx) {
    const dispatcher = ctx.use<Services['dispatcher']>('dispatcher')
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

仓库的 fixture 展示了两种语言：`runtime/fixtures/branch-part.ts` 和
`runtime/fixtures/python/python_branch_part.py`。
