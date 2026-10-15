# ContextCompaction

状态：草案（待评审）。路线图项 F1（[#72](https://github.com/arcships/dim/issues/72)）。
总体设计：[附录 B.1 ContextPreparation 行](../overview.md#b1-session-与分支-capability)。
作用域：[ADR 002](../../adr/002-plugin-scope.md)：两个 Session 部件，加上对现有分支 Capability Loop 与
ContextPreparation 的补充。本文内容均未实现。暂定值（触发点、保留量）在
[#128](https://github.com/arcships/dim/issues/128) 研究。英文版：[ContextCompaction](../../../design/plugins/ContextCompaction.md)。

## 1. 概要

**压缩**是把一个切点之前的历史做成摘要，范围通常是上一个压缩点以来的全部内容加上一份摘要，并原样保留最后几轮。此后分支在新结果上推进。压缩运行期间 Loop 不推进：压缩是 Loop 推进过程中的一步。否则压缩期间上下文可能溢出，新事件也会基于旧上下文决定，与摘要不一致。

自动压缩已经如此。它在请求的准备步骤里运行（`context/prepare` → `context/compaction` → `model/call`），Loop 等待该请求的终态（[context_policy.rs](../../../../crates/dim-agent-capabilities/src/context_policy.rs)）。但它在生产环境里从不运行，因为 App Server 绑定的是 `ContextModelHost(None)`（[main.rs](../../../../crates/dim-app-server/src/main.rs)）。这种绑定下 ContextPreparation 没有策略，Loop 也没有预算恢复。

F1 新增：

1. **生产宿主。** Session 部件 `ProductContextModel` 提供 `ContextModelHost(Some(..))`。它在两个现有进程服务之上实现现有端口 `ResolveContextModel` → `ContextModel`（[host.rs](../../../../crates/dim-agent-capabilities/src/host.rs)）：`ProviderConnectionResolver`（连接与凭据）和 `ModelHost`（Aimux，执行）。
   - 压缩用请求自己的模型。
   - 上下文窗口和输出预算来自请求上已采用的能力快照。
   - token 计数是本地估算。
2. **两种触发共用一条切点规则。** 由 ContextPolicy 选切点：最后 K 轮之前紧挨着的合法切点（暂定 K = 2，#128）。摘要范围是从上一个压缩点到该切点，含上一份摘要。
3. **手动压缩是 Loop 指令。** `session.context.compact` 写指令事件 `context/compact`，由 Loop 按序消费：
   - 空闲时，Loop 运行一个仅压缩 turn：一个准备步骤，不调模型。
   - turn 进行中时，Loop 在下一个请求的准备中、该请求调模型之前强制压缩。
   - 两种情况都走 ContextPreparation 和 ContextPolicy，与自动压缩同一条路径；压缩事实带 Loop 真实的 turn 与 request id。
4. **用量查询。** `session.context.usage` 返回此刻策略会算出的预算，不写任何东西。

两个方法由第二个 Session 部件 `ContextMethods` 经 `sessionMethods`（命名空间 `session.context`）提供。

F1 不动的部分：

- Provider、Aimux、ModelProvider、Timeline、Dispatcher、事件信封和 rutis 都不改。
- F1 新增一个指令事件，并在 `context/prepare` 上加一个可选字段（§9；格式决定见 §10）。
- 不新增配置，也不新增进程插件。

### 1.1 为什么这么小

- **不用 AuxiliaryModel。** S4 的 `auxiliaryModel.complete` 只返回文本。压缩需要被压缩上下文所属模型的窗口、输出预算和 token 计数（[auxiliary-model §1.1](https://github.com/arcships/dim/blob/roadmap/s4-provider-purpose-policy-and-auxiliarymod/docs/design/appserver/auxiliary-model.md)）。
- **不放进 Provider。** 宿主需要的东西都能经 `dim-agent-capabilities::host` 的通用端口拿到：
  - 能力快照随 `ModelRequest` 传递。
  - 凭据由 `ProviderConnectionResolver` 解析，与 ModelProvider 每个 turn 的做法完全相同。
  - 执行用 `ModelHost`。
- **没有第二条压缩路径。** 手动压缩复用请求的准备步骤。Loop 只多出指令、待处理标记和仅压缩 turn。

## 2. 结构

```
 进程作用域（现有，不变）
 +-------------------------------------------------------------------+
 |  Aimux: ModelHost        Provider: ProviderConnectionResolver     |
 +-----------------^-------------------------------^-----------------+
                   | 注入（硬依赖）                | 注入（硬依赖）
 Session 作用域    |                               |
 +-----------------+-------------------------------+-----------------+
 |  ProductContextModel（新）                                        |
 |    提供 S:ContextModelHost(Some(ProductContextResolver))          |
 |  ContextMethods（新）                                             |
 |    提供 sessionMethods["session.context"]: compact, usage         |
 |    每次调用: TreeCommands.get(branchId)                           |
 |      compact -> .service::<AgentLoop>().compact()                 |
 |      usage   -> .service::<ContextPreparation>().usage()          |
 +---------------------------------^---------------------------------+
            Rpc（C1）分发 ---------'
 分支作用域（一个 Timeline，一个 Dispatcher）
 +-------------------------------------------------------------------+
 |  Loop（补充）                                                     |
 |    拥有 context/compact；待处理标记；仅压缩 turn；                |
 |    以现有交接写 context/prepare {compact}                         |
 |        | context/prepare                     ^ context/compaction |
 |        v                                     | step/model-output-end
 |  ContextPreparation（补充）  ContextPolicy：阈值、恢复、手动共用  |
 |    一条压缩路径；切点在最后 K 轮之前；                            |
 |    写 context/budget、context/compaction、model/call              |
 |  ModelProvider（不变）: model/call -> ModelHost                   |
 +-------------------------------------------------------------------+
```

两个 Session 部件分开，是因为依赖不同。分支注入 `ContextModelHost`，它的提供者不能依赖由分支树提供的 `TreeCommands`。两者都放在 `dim-agent-capabilities`。所有新依赖都从新部件指向现有核心服务。

## 3. 数据与状态

| 项 | 所有者 | 类别 |
| --- | --- | --- |
| `context/compact {}` | Loop | 新指令事件（§4.3） |
| `context/prepare.compact: "before_model" \| "only"` | Loop 写，ContextPreparation 消费 | 现有事件的新可选字段（§4.3） |
| `context/compaction` | ContextPreparation | 现有事实、现有格式；手动压缩时 `diagnostics.trigger: manual` |
| 待处理压缩：最近一条未服务的 `context/compact` 的序号 | Loop | 私有，由已消费事件折叠，装载时重建（§5.4） |
| 已服务压缩：最近一次已服务指令的 `{command, outcome}` | Loop | `StateCell`，由同样的事件折叠；ContextMethods 读取它来回复 |
| 仅压缩请求的阶段 `Compacting` | Loop | 私有请求阶段，由 `context/prepare {compact: "only"}` 重建 |

宿主不持有状态。解析出的连接只存在于一次 `compact` 调用内。

## 4. 接口

### 4.1 Session 方法（语言中立，ADR 002 D5）

`sessionMethods`，Session 作用域，键 `session.context`，由 ContextMethods 提供（[session-boundary A.1](../contracts/session-boundary.md#a1-方法贡献)）。两个方法都声明为 `interruption: report`、`progress: false`、`notifications: []`、`reply: direct`。

| 方法 | 变更 | 请求 | 回复 |
| --- | --- | --- | --- |
| `session.context.compact` | 是 | `{sessionId, branchId}` | `{status: "completed", sequence}` 或 `{status: "skipped", reason}` |
| `session.context.usage` | 否 | `{sessionId, branchId}` | `{through, contextWindow, outputTokens, thresholdTokens, estimatedInputTokens, baselineSource: "provider_baseline" \| "full_estimate"}` |

`session.context.compact`：

- 经 Loop 写 `context/compact`，等 Loop 服务完这条指令后回复（§5.3）。
- `sequence` 是已提交的 `context/compaction` 事件。
- `skipped` 带策略给出的原因，例如历史在保留的几轮之前没有合法切点时为 `nothing_to_compact`。
- 一条指令被服务之前写入的多条指令一起被服务，每个调用方得到相同的结果。

`session.context.usage`：

- `through` 是计算这些数值时的已提交头部。
- 数值等于在该头部做一次准备时写进 `context/budget` 的值，外加 `thresholdTokens`。

取消按 session-boundary A.1：卸载会中断调用，调用方断开不取消任何东西。已被服务但没有压缩成的指令回复错误（§6）。

### 4.2 ContextModel 端口（现有，Rust）

`ContextModel` 是 Rust 运行时内部现有的宿主端口，不是插件合同。F1 给它的 `CompactionInput` 加两个字段，都由生产者 ContextPolicy 设置，宿主不决定它们：

- `cut: String`：宿主摘要到哪一条历史条目为止（§5.2）。
- `trigger: CompactionTrigger`：`Threshold`、`ProviderOutputBudgetExhausted` 或 `Manual`。

### 4.3 事件

| 事件 | 写入方 | 消费方 | 数据 |
| --- | --- | --- | --- |
| `context/compact`（新） | Loop，经其公开写操作 `AgentLoop::compact` | Loop，按序号 | `{}` |
| `context/prepare`（加字段） | Loop | ContextPreparation | 现有数据，外加可选的 `compact: "before_model" \| "only"`，没有时省略 |

关于新事件与新字段：

- `context/compact` 是发给推进过程所有者的指令（原则 §4 第 1 条）。它的结果是 Loop 和 ContextPreparation 响应时写的事实（§5.3）。它对客户端不可见：`tree_protocol.rs` 中穷举的 `ProductEvent` 分类把它和其他内部事件列在一起。
- `compact: "before_model"`：先压缩，再把压缩后的请求交给 ModelProvider（`model/call`）。
- `compact: "only"`：压缩后停止。已提交的 `context/compaction` 事实就是该请求的终态。

### 4.4 Rust 绑定

```rust
// dim-agent-types/src/events.rs（增量）
EventKind::ContextCompact(ContextCompactData {})         // "context/compact"
pub struct ContextPrepareData { /* 现有 */ pub compact: Option<CompactStep> }
pub enum CompactStep { BeforeModel, Only }               // "before_model" | "only"

// dim-agent-capabilities/src/host.rs（同一 crate 的现有端口）
pub struct CompactionInput { /* 现有 */ pub cut: String, pub trigger: CompactionTrigger }

// dim-agent-capabilities/src/agent_loop.rs（补充）
impl AgentLoop {
    /// 追加 context/compact；Loop 服务完后返回。
    pub async fn compact(&self, cancel: CancellationToken) -> Result<CompactionServed>;
}
pub enum CompactionServed { Completed { sequence: u64 }, Skipped { reason: String },
                            Failed { error: String }, Cancelled { reason: String } }
impl PublicService for AgentLoop {}                       // lib.rs

// dim-agent-capabilities/src/context_preparation.rs（补充）
impl ContextPreparation {
    pub async fn usage(&self, cancel: CancellationToken) -> Result<ContextUsage, ContextOpError>;
}
impl PublicService for ContextPreparation {}             // lib.rs

// dim-agent-capabilities/src/context_model.rs（新）
pub struct ProductContextModelPlugin { scope: SessionScope }
//   Deps = (Arc<ModelHost>, Arc<ProviderConnectionResolver>)  -- 进程作用域
//   提供 scope.key::<ContextModelHost>() = ContextModelHost(Some(ProductContextResolver))
// dim-agent-capabilities/src/context_methods.rs（新）
pub struct ContextMethodsPlugin { scope: SessionScope }
//   Deps = InSession<TreeCommands>；SessionMethods::provide(ctx, &scope, "session.context", ..)

// dim-app-server/src/main.rs，仅产品配置（没有 --models 文件）：
builder.session_plugin(ProductContextModelPlugin::new)
       .session_plugin(ContextMethodsPlugin::new)
       .session_methods("session.context");
// 在这里替换 .session_value(Arc::new(ContextModelHost(None)))；
// --models 与 mock 配置仍是 ContextModelHost(None)
```

`SessionMethods` 是 C1 放在 `dim-agent-capabilities::methods` 的类型。

## 5. 流程

### 5.1 生产宿主

`ProductContextResolver::resolve(request)`：

1. 要求 `request.capability_snapshot` 存在且对请求的模型有效。没有时返回 `InvalidRequest`“Model capabilities are not adopted; select the model again”，条件和文本与 `ProductResolver` 相同，那里本来就会拒绝这种请求。
2. 返回绑定该请求的 `ContextModel`：

| 方法 | 值 |
| --- | --- |
| `context_window` | `snapshot.capabilities.contextWindow`，即目录值，含目录现有的覆盖 |
| `output_tokens` | 有 `request.config.maxTokens` 时取它，否则取 `snapshot.capabilities.maxOutputTokens`；不超过后者 |
| `threshold_tokens` | `0.8 × (context_window − output_tokens)`，暂定（#128） |
| `count_tokens` | 本地估算：序列化后的系统提示、消息和工具声明的 UTF-8 字节数除以 4；每个图片部件按固定 1,600 token 计，而不按编码字节计。暂定（#128）。策略只在以下场合用它：在提供方报告的 `prompt_tokens` 之上算增量、第一个请求、`validate`、以及检查压缩是否减少了输入 |
| `compact` | 见下 |

`compact(input, cancel)` 摘要 `input.checkpoint` 中的摘要加上直到并包括 `input.cut` 的历史条目，返回 `Completed {cursor: input.cut, summary, diagnostics}`：

- **请求。** 每块一个摘要请求。用请求的 provider、model、快照和 config，`maxTokens = output_tokens`，系统提示是本模块拥有的固定摘要提示，一条包含记录文本的 user 消息，不带工具。
- **凭据与失败。** `ProviderConnectionResolver.resolve` 返回只在请求内使用的连接。调用经 `invoke_with_recovery`（`dim-agent-capabilities::model_provider`）运行，它就是 ModelProvider 使用的失败与凭据策略。S4 抽出了这个函数；如果 F1 先合入，由 F1 做同样的抽取。
- **输出。** 丢弃 thinking 输出。工具停止或空文本算失败。
- **记录文本。** 每条按角色标注成文本：工具调用写名称和输入，工具结果写其文本，图片写 `[image]`。
- **分块。** 单条超过一次调用输入预算（窗口 − 输出 − 提示）的，从中间截断，计入 `compactedToolPayloadCount`。一次调用放不下的记录分块折叠：每次调用拿到当前摘要和下一块。
- **诊断。** `CompactionDiagnosticsRecord` 的必填字段来自本次运行（`reason` 和 `trigger` 取自 `input.trigger`，`coverage: full`，各计数、估算和 `durationMs`）。宿主不计算的可选字段省略。

### 5.2 一条压缩路径（ContextPolicy）

ContextPolicy 现有的 `prepare` 只改两处。其余保持现状：估算、`context/budget`、重试时复用检查点、验收压缩后的请求更小且低于窗口、提交 `context/compaction`、可选快照。

**何时压缩。** `source.recovery` 或 `source.compact` 存在时强制压缩。否则同现在一样，每个边界估算达到阈值时压缩一次。有 `recovery` 时触发原因为 `ProviderOutputBudgetExhausted`，否则有 `compact` 时为 `Manual`，否则为 `Threshold`。

**切点。** 由 ContextPolicy 而不是宿主选切点，两种触发遵循同一条规则：

- 保留的是分支上的最后 K 轮，含正在进行的那一轮。一轮从 `turn/start` 到 `turn/end`。
- 条目按其来源事件在固定上下文中的序号归属到某一轮。从快照状态恢复的条目早于该快照锚定的压缩，算作比任何保留轮都早。
- 切点是最老保留轮的第一条之前最晚的合法切点。合法切点指通过现有检查的切点：不保留被移除调用的工具结果，下一条也不是工具上下文。
- 没有这样的切点时：在 `recovery` 下退回到最晚的合法切点，让请求仍然放得下；否则结果是 `Skipped {reason: "nothing_to_compact"}`，不调模型。
- 因此摘要范围是从上一个压缩点到切点，含上一份摘要。切点之后的 K 轮原样保留，按键的基线块照旧从切点处的权威基线重放。

**仅压缩请求。** 对 `compact: "only"`，已提交的 `context/compaction` 事实（`Completed`、`Skipped` 或 `Failed`）就是请求的结果。ContextPreparation 不写 `model/call`（§5.3）。

### 5.3 手动压缩

Loop 用它准备的下一个请求来服务待处理的 `context/compact`。如果 Loop 空闲且没有输入驱动 turn，它启动一个仅压缩 turn。

**空闲分支：**

```
 序号 事件                                               写入方
 n    context/compact {}                                 Loop（AgentLoop::compact，来自
                                                           session.context.compact）
      -- Loop 消费 n：空闲，无驱动输入 -> 仅压缩 turn
 n+1  turn/start {T}                                     Loop
      -- Loop 消费 n+1：不写 input/consumed，不写 step/model-start
 n+2  context/prepare {T, R, through: n+1, request,      Loop（交接给 ContextPreparation）
                       compact: "only"}
 n+3  context/budget {T, R, ...}                         ContextPreparation
 n+4  context/compaction {T, R, through: n+1, outcome}   ContextPreparation  <- R 的终态
      -- Loop 消费 n+4：清除待处理标记，记录已服务结果
 n+5  turn/end {T, completed "compacted"                 Loop
                 | completed "compaction_skipped"
                 | failed <error>}
      -- n+1 之后应用的驱动输入启动下一个 turn（try_start）
```

如果准备在提交事实之前失败或被取消，ContextPreparation 写现有终态 `step/model-output-end {T, R, Failed | Cancelled}`，代替 n+3/n+4。Loop 随后以 `failed` 或 `cancelled` 写 `turn/end`。

**turn 进行中：**

```
      step/model-start {T, R1} ... step/model-output-end {T, R1, completed}
 k    context/compact {}                                 -- Loop：待处理 = k
      step/tool-call {T, R1} ... step/tool-result {T, R1}
      input/consumed* ; step/model-start {T, R2}         Loop（不变）
      context/prepare {T, R2, through, request,          Loop：写入时仍有待处理标记
                       compact: "before_model"}
      context/budget {T, R2}                             ContextPreparation
      context/compaction {T, R2, outcome}                ContextPreparation
      -- Loop 消费它：清除待处理标记，记录已服务结果
      model/call {T, R2}（基于压缩后的请求）             ContextPreparation -> ModelProvider
```

如果 turn 在准备下一个请求之前就结束，Loop 转为空闲时待处理标记仍在，于是按上面的方式由仅压缩 turn 服务。`before_model` 请求中的手动压缩失败时，请求不压缩继续，与自动压缩相同。

**回复。** `AgentLoop::compact` 追加 `context/compact`，等到 Loop 的已服务状态覆盖该序号。ContextMethods 映射结果：

- `Completed` → `{status: "completed", sequence}`
- `Skipped` → `{status: "skipped", reason}`
- `Failed` → `compaction_failed`
- `Cancelled` → `compaction_cancelled`

### 5.4 Loop 新增的内容

全部在 `crates/dim-agent-capabilities/src/agent_loop.rs`。

| 方面 | 改动 |
| --- | --- |
| 订阅 | 在现有列表中加入 `context/compact` 与 `context/compaction` |
| 公开写操作 | `AgentLoop::compact(cancel)` 追加 `context/compact` 并等待已服务状态；标为 `PublicService`，使 ContextMethods 能按分支找到它 |
| 状态 | `compaction: Option<u64>`，即待处理指令；`served: StateCell<..>`；`Stage::Compacting`；在该 turn 的请求存在之前用 `Turn.compaction_only` 标记 |
| 消费 `context/compact` | 设置待处理标记（之后的指令替换序号，因此多条一起服务）。空闲时调用 `try_start` |
| `try_start` | 有驱动输入时（不变），或有待处理压缩时启动 turn。没有驱动输入时该 turn 为仅压缩 turn |
| 消费 `turn/start` | 仅压缩 turn 不调用 `next_model`，而是启动压缩步骤：不写 `input/consumed`，不写 `step/model-start`，请求阶段为 `Compacting`，`start_preparation(through = turn/start 的序号)` |
| `start_preparation` | 仅压缩请求设 `compact: "only"`。写 `context/prepare` 的那一刻若有待处理压缩（在 turn 锁内，steer 与中断检查也在这里），设 `compact: "before_model"`。预算重试复制 source 时清除 `compact`，因为事实已经服务过它 |
| 消费 `context/compaction` | 对其 `context/prepare` 带 `compact` 的请求：清除待处理标记，已服务结果设为 `Completed {sequence}`、`Skipped` 或 `Failed`。若该请求处于 `Compacting`：释放它并写 `turn/end`，结果为 `completed "compacted"`、`completed "compaction_skipped"`，或以事实中的错误为 `failed` |
| `Compacting` 请求的 `step/model-output-end` | `Failed` 或 `Cancelled` 以同样的结果结束 turn（不做预算恢复），清除待处理标记，已服务结果为 `Failed` 或 `Cancelled` |
| 带 `compact` 的请求的其他终态 | 在其事实之前到达的任何终态（`replaced by steer` 除外）都清除待处理标记，已服务结果为 `Failed` 或 `Cancelled`。被 steer 替换后标记保留，由下一个请求带上 |
| `steer` | 跳过 `Compacting` 请求：驱动输入保持待处理，在 `turn/end` 之后启动下一个 turn。压缩期间 Loop 不推进 |
| 消费 `stop/interrupt` | 清除待处理标记，已服务结果为 `Cancelled`。正在运行的仅压缩准备经现有取消路径被取消 |
| `rebuild` | 把 `context/compact`、各请求的 `compact` 字段、`context/compaction` 和起清除作用的终态折叠成待处理与已服务状态。`context/prepare {compact: "only"}` 重建出一个 `Compacting` 请求。未结束的仅压缩 turn 与任何重建的 turn 一样被标为失败：ContextPreparation 以重载终态结算其未完成的准备，然后 Loop 结束该 turn。还没有 `context/prepare` 的仅压缩 turn 没有未完成的工作，立即结束 |

不变的部分：turn 与 request id、输入消费、工具批次、模型请求的预算恢复、`LoopSnapshot`。仅压缩 turn 以 `Running` 发布，因此期间到达的输入与任何 turn 期间完全一样地排队。

### 5.5 ContextPreparation 新增的内容

| 方面 | 改动 |
| --- | --- |
| `context/prepare {compact}` | 把 source 交给 ContextPolicy（§5.2）。对 `"only"`：成功时不写 `model/call`，因为策略提交的事实就是终态；`Failed` 和 `Skipped` 结果是事实，不是错误。事实之前的错误照旧写 `step/model-output-end` |
| Steer | 规则不变。仅压缩请求没有 `step/model-start`，它的 steer 标记永远不匹配，steer 永远不会取消它 |
| `rebuild` | 未完成的 `context/prepare {compact: "only"}` 也可由它自己的 `context/compaction` 结算 |
| `usage(cancel)` | 读取已提交头部，按 Loop 的方式由最新 `context/configured` 的设置和 system 加投影后的消息构造请求，不绑定工具。解析 `ContextModel` 并运行现有估算。不写任何东西；可以在 turn 进行中调用 |

## 6. 错误

`MethodError` 代码（未注明的为 `code -32000`）：

| 代码 | 方法 | 何时 | 重试 |
| --- | --- | --- | --- |
| `-32602` | 两者 | 缺少 `branchId` 或不是字符串 | 否 |
| `session_invalid_input` | 两者 | 未知分支（`AgentError::UnknownBranch` 的现有映射） | 否 |
| `session_closed` | 两者 | Session 或分支正在关闭 | 否 |
| `context_not_configured` | usage | 分支历史中没有 `context/configured` | 采用模型之后 |
| `context_model_failed` | usage | `resolve` 失败（没有能力快照） | 重新选择模型之后 |
| `compaction_failed` | compact | 已服务结果为 `Failed`：共享失败策略之后的宿主错误、空摘要、非法切点、没有降到窗口以下，或压缩前的准备失败；消息为错误文本 | 由用户决定 |
| `compaction_cancelled` | compact | 已服务结果为 `Cancelled`：`stop/interrupt`，或带标记的请求被取消 | 由用户决定 |
| `operation_interrupted` | compact | 调用等待期间 Loop 或 ContextMethods 卸载（session-boundary A.1，`report`）；指令仍在 Timeline 中，重载时被折叠 | 由用户决定 |

自动压缩的错误处理不变。宿主错误成为事实中的 `Failed` 结果，请求不压缩继续；在恢复中则使请求失败。

## 7. 安全

- **凭据只在请求内。** 连接在一次 `compact` 调用内从 `ProviderConnectionResolver` 取得并交给 `ModelHost`，与 turn 相同。它从不被保存、返回、序列化或记录。Provider 不变。
- **没有新的数据外发。** 摘要调用把 Session 自己的历史发给每个 turn 都会收到它的同一个提供方和模型。
- **摘要是不可信的模型输出。** 它只经现有的 `summary_message` 进入上下文，被标为数据并转义。
- **结果中没有密钥。** 事实、回复和诊断只含计数、id 和 `HostError` 文本，该文本不含凭据值。请求和摘要内容不进 tracing。
- **授权。** Rpc 对 Session 方法校验 `sessionId`（C1）。能调用 Session 方法的连接都可以压缩，正如它可以提交输入或停止。

## 8. 与现有部件的关系

| 部件 | 关系 |
| --- | --- |
| Loop | 补充（§5.4）：指令、待处理标记、仅压缩 turn。产品配置下 `ContextModelHost` 为 `Some`，预算恢复生效 |
| ContextPreparation、ContextPolicy | 补充（§5.2、§5.5）：所有触发共用一条压缩路径、K 轮切点、仅压缩请求、`usage` |
| ModelProvider | 代码不变。产品配置下 `validate` 开始按估算检查窗口；它永远收不到仅压缩请求 |
| Provider、ProductResolver、Aimux / ModelHost | 不变；只经 `ProviderConnectionResolver` 和 `ModelHost` 使用 |
| AuxiliaryModel（S4） | 不使用。共用 `invoke_with_recovery`；S4 与 F1 谁先合入谁负责抽取 |
| `sessionMethods`（C1） | 两个方法需要它。宿主部件不需要 C1，可以先合入 |
| Steer（ADR agent-029） | 对模型请求不变。仅压缩请求不可被 steer |
| turn 的消费方（客户端、将来的 Goal、SessionTitle） | 把仅压缩 turn 看作一个普通 turn，带自己的结果原因（§10） |
| `tree_protocol.rs` 的 `ProductEvent` | 把 `context/compact` 归为客户端不可见（穷举匹配） |
| HistoryRenderer、快照、原生存储 | 投影不变；手动事实与自动事实一样被投影和做快照。新事件类型与新字段需要格式决定（§10） |
| `--models` 配置、mock 配置 | 保持 `ContextModelHost(None)`：不压缩，也不登记 `session.context` |
| ManagedProviders 套餐用量、CLI `/compact` 与 `/usage` | 不在 F1 的验收内；CLI 调用这两个方法 |

## 9. 规则检查

| 规则 | 回答 |
| --- | --- |
| 原则 §1（Capability、App Server 边界） | 两个 Session Capability 和两处补充都在 `dim-agent-capabilities`；App Server 只在产品配置中安装这些部件。 |
| §1 唯一事件总线 | 手动压缩是分支 Timeline 上的指令，由分支 Dispatcher 消费。没有第二条总线，也没有旁路；方法的回复读取 Loop 的 `StateCell`。 |
| §3 依赖方向 | 新部件依赖核心服务（`ModelHost`、`ProviderConnectionResolver`、`TreeCommands`、Loop、ContextPreparation）。现有部件只经现有的 `ContextModelHost` 键用到它们，而安装该部件的地方总会提供这个键。没有可选依赖。 |
| §3 改动其他模块（Loop） | 已批准的改动。需求来自 Loop 自己的上下文管理：压缩必须是 Loop 推进中的一步，Loop 才不会在一个将被摘要取代的上下文上推进。它在 Loop 内闭环：自动压缩本来就在 Loop 的准备步骤里，手动压缩由同一个步骤服务，要么在下一个请求中，要么在一个仅压缩 turn 中。没有其他模块因此改变形状。 |
| §4 规则 1（指令事件） | 客户端用 `context/compact` 请求推进过程的所有者。Loop 决定何时服务它，结果事实由各自的所有者写：Loop（`turn/*`、`context/prepare`）和 ContextPreparation（`context/budget`、`context/compaction`）。被中断清除的指令不写事实，由调用方的回复说明。 |
| §4 规则 2（交接） | 压缩请求经 `context/prepare` 交给 ContextPreparation，这是现有的交接。此后 Loop 只能用 `stop/interrupt`。 |
| §4 规则 3（before 回调；处理器） | 不用 before 回调。Loop 的处理器与现在一样登记准备任务后返回。 |
| §4 规则 4（核心无业务含义） | Timeline、Dispatcher 和运行时门面不变。 |
| §4 规则 5（原子批次） | 每次新写入都是一个事件；`turn/end` 保持现有批次。 |
| §4 规则 6（上下文只来自交接后的事件） | 摘要只经已完成事实的现有投影到达模型。仅压缩步骤不消费待处理输入，所以它们永远不会落入被摘要的范围（AGENTS 规则 10）。 |
| §4 规则 7（公开事件与服务） | `TreeCommands`、`PublicService`（`AgentLoop`、`ContextPreparation`）上的 `Branch::service`、`ProviderConnectionResolver`、`ModelHost`、`sessionMethods`。 |
| §4 规则 8（基础设施不变） | Timeline、Dispatcher、信封、rutis、loader、Agent 装配和门面都不改。Loop 是 Capability，不在所列基础设施之内。新事件类型是现有信封里的插件内容。不向 rutis 提需求。 |
| §4 规则 9（只写需要的事实） | `context/compact` 是需要的：Loop 后续的决定要读它，重载时也要折叠它。`context/compaction` 用于投影。用量查询不写任何东西。 |
| ADR 002 D1（作用域） | 宿主与方法：Session。指令、待处理标记和压缩步骤：分支，放在推进过程的所有者中。切点与事实：分支，放在压缩事实的所有者中。 |
| ADR 002 D2（无接线模块） | ContextMethods 经规定的 `sessionMethods` 暴露所有者的操作；ProductContextModel 是本功能的宿主。 |
| ADR 002 D3（依赖） | 只硬依赖核心。Provider 或 Aimux 重载会像传播到 ModelProvider 一样传播到这些部件（已接受的代价）。 |
| ADR 002 D4（配置） | 不增加配置；暂定常量写在代码里（#128）。 |
| ADR 002 D5（语言中立合同） | §4.1 与 §4.3 是合同；§4.4 是一种绑定。 |

**影响面（新增公开内容）：**

- `session.context` 命名空间（`compact`、`usage`）：`/compact` 和 `/usage` 需要它。
- 指令事件 `context/compact`：手动压缩必须进入 Timeline 并由 Loop 消费。
- 可选字段 `context/prepare.compact`：Loop 必须告诉 ContextPreparation 要压缩，以及压缩后是否停止。

内部 Rust 改动：

- `CompactionInput.cut` 与 `CompactionInput.trigger`：由生产者选定。
- `AgentLoop` 与 `ContextPreparation` 上的 `PublicService`：让 Session 部件按分支找到它们。

**向机制所有者提出的需求：** 无。

## 10. 决定与待定问题

已决定：

| 项 | 决定 |
| --- | --- |
| 用哪个模型压缩 | 请求自己的模型；单独的压缩模型推迟 |
| 触发点与保留量 | `0.8 × (窗口 − 输出)`、K = 2 轮、`字节数 / 4`、每张图片 1,600 token：暂定，在 #128 研究 |
| 手动压缩 | 一条在 Loop 推进中服务的 Loop 指令，带真实的 turn 与 request id（§5.3） |
| `--models` 配置 | 没有能力快照，因此不压缩：接受 |
| 提供方报告的溢出 | Aimux 报告时不带预算事实；映射它是单独的 Aimux issue，不在 F1 内 |

待定（由 Loop 改动引出）：

1. **格式决定。** `context/compact` 是新的事件类型，`context/prepare.compact` 是 `deny_unknown_fields` 类型上的新可选字段。两者都是增量，但旧版程序读不了包含它们的分支。原生存储格式需要升版本吗，还是把增量事件类型记为兼容（S1 对 `input/withdraw` 也要做同样的决定）？
2. **仅压缩 turn 作为 turn。** 它以 `turn/start` 和 `turn/end`（`completed "compacted"`）发布，客户端和将来的 turn 消费方（Goal 在 `turn/end` 上决定续跑、Hooks 的 turn 结束 Hook）都会看到它。可以接受吗？还是消费方应跳过结果原因为 `compacted` 的 turn？
3. **强制压缩期间的 steer。** 在 `before_model` 请求中，应用的 steer 输入仍会取消准备，包括正在进行的压缩，与现在的自动压缩一样。下一个请求带着待处理标记，在更新后的历史上重新压缩。保持这样，还是让正在压缩的准备都不可被 steer？

## 11. 验收（流程测试）

测试用产品配置的 App Server 连接一个 mock HTTP 提供方，其目录条目的上下文窗口很小（`crates/dim-app-server/tests`），另加 `crates/dim-agent-capabilities/tests` 中的 Loop 流程测试。

1. **自动。** 对话增长超过阈值。
   - 下一次准备写 `context/budget`，并发出一个摘要请求：摘要系统提示、一条 user 消息、不带工具、Session 的模型。
   - 提交 `trigger: threshold` 的 `completed` `context/compaction`，游标是最后 2 轮之前的合法切点。
   - 模型请求以摘要开头，原样保留最后 2 轮，不再包含被摘要的消息。
2. **预算恢复。** 估算达到窗口的请求以预算事实结束。Loop 重试一次，重试时以 `trigger: provider_output_budget_exhausted` 压缩（只有最后 2 轮时退回到最晚的合法切点）。
3. **手动，空闲。** `session.context.compact` 产生的序列正好是 §5.3 那样：
   - `context/compact`、`turn/start`、`context/prepare {compact: "only"}`、`context/budget`、`context/compaction`、`turn/end completed "compacted"`。
   - 没有 `step/model-start`，没有 `model/call`，除摘要请求外没有别的模型请求。
   - 回复为 `completed` 并带事实的序号，事实中的 id 属于这个 turn。
   - 压缩期间提交的输入在 `turn/end` 之后启动新 turn，其请求以摘要开头。
4. **手动，turn 进行中。** 在工具批次运行时写入指令。下一个请求的 `context/prepare` 带 `compact: "before_model"`，它的 `context/compaction` 先于它的 `model/call`，回复在事实之后到达。
5. **中断与重载。**
   - 仅压缩 turn 期间 `session.stop`：turn 以 `cancelled` 结束，不写压缩事实，回复 `compaction_cancelled`。
   - 期间重启 App Server：该 turn 以重载原因 `failed` 结束，不重发摘要请求。
6. **用量。** `session.context.usage` 返回目录窗口、输出预算、阈值和估算。压缩之后估算下降，调用后分支 Timeline 没有新事件。
7. **失败与凭据。**
   - 提供方让摘要请求失败时，自动路径提交 `failed` 并不压缩继续，手动回复 `compaction_failed`。
   - 事实、回复和诊断中不出现 API key。
