# ContextCompaction

状态：草案（待评审）。路线图项 F1（[#72](https://github.com/arcships/dim/issues/72)）。
总体设计：[附录 B.1 ContextPreparation 行](../overview.md#b1-session-与分支-capability)。
作用域：[ADR 002](../../adr/002-plugin-scope.md)：两个 Session 部件，加上对现有分支 Capability Loop 与
ContextPreparation 的补充。本文内容均未实现。触发点和保留量是暂定值，在
[#128](https://github.com/arcships/dim/issues/128) 研究。英文版：[ContextCompaction](../../../design/plugins/ContextCompaction.md)。

## 1. 概要

**压缩**是把切点之前的历史做成摘要，并原样保留最后几轮。摘要范围通常是上一个压缩点以来的全部内容，加上上一份摘要。此后分支在新结果上推进。

压缩是 Loop 控制上下文窗口的一部分。它是 Loop 的一个步骤，和模型调用、工具批次一样，不绑定 turn。压缩步骤运行期间，Loop 不推进到下一步。否则压缩期间上下文可能溢出，新事件也会基于旧上下文决定，与摘要不一致。

自动压缩已经在请求的准备中运行（`context/prepare` → `context/compaction` → `model/call`），Loop 等待该请求的终态（[context_policy.rs](../../../../crates/dim-agent-capabilities/src/context_policy.rs)）。但它在生产环境里从不运行，因为 App Server 绑定的是 `ContextModelHost(None)`（[main.rs](../../../../crates/dim-app-server/src/main.rs)）。

F1 新增：

1. **生产宿主。** Session 部件 `ProductContextModel` 提供 `ContextModelHost(Some(..))`。它在两个现有进程服务上实现现有端口 `ResolveContextModel` → `ContextModel`（[host.rs](../../../../crates/dim-agent-capabilities/src/host.rs)）：`ProviderConnectionResolver`（连接与凭据）和 `ModelHost`（Aimux）。
   - 压缩用请求自己的模型。
   - 窗口和输出预算来自请求上已采用的能力快照。
   - token 计数是本地估算。
2. **一条压缩路径、一条切点规则。** ContextPolicy 的压缩步骤是一个函数，自动压缩在准备中用它，Loop 的手动步骤也用它。切点是最后 K 轮之前紧挨着的合法切点（暂定 K = 2，#128）。
3. **手动压缩是 Loop 的一个步骤。** `session.context.compact` 写指令 `context/compact`。Loop 消费它并安排一个压缩步骤：
   - 在 turn 内，步骤在下一个步骤边界运行，即准备下一个请求之前。之后 turn 从压缩后的上下文继续。
   - 没有 turn 在运行时，步骤立即运行。它写 `context/compaction`，除此之外什么都不发生：没有 turn。
4. **用量查询。** `session.context.usage` 返回此刻策略会算出的预算，不写任何东西。

两个方法由第二个 Session 部件 `ContextMethods` 经 `sessionMethods`（命名空间 `session.context`）提供。

不变的部分：Provider、Aimux、ModelProvider、Timeline、Dispatcher、事件信封、rutis 和 `context/prepare`。F1 新增一个事件类型 `context/compact`，并直接修改 `context/compaction`；软件尚未发布，接受破坏性修改（§4.3）。

### 1.1 为什么这么小

- **不用 AuxiliaryModel。** S4 的 `auxiliaryModel.complete` 只返回文本。压缩需要被压缩上下文所属模型的窗口、输出预算和 token 计数（[auxiliary-model §1.1](https://github.com/arcships/dim/blob/roadmap/s4-provider-purpose-policy-and-auxiliarymod/docs/design/appserver/auxiliary-model.md)）。
- **不放进 Provider。** 宿主需要的东西都能经 `dim-agent-capabilities::host` 的通用端口拿到：
  - 能力快照随 `ModelRequest` 传递。
  - 凭据由 `ProviderConnectionResolver` 解析，与 ModelProvider 每个 turn 的做法相同。
  - 执行用 `ModelHost`。
- **没有第二个状态机。** 所有步骤由 Loop 排序。ContextPreparation 只在 Loop 已经交给它的请求准备中运行压缩函数。

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
 |      compact -> .service::<AgentLoop>().compact()，然后等待引用    |
 |                 该指令的 context/compaction                       |
 |      usage   -> .service::<ContextPreparation>().usage()          |
 +---------------------------------^---------------------------------+
            Rpc（C1）分发 ---------'
 分支作用域（一个 Timeline，一个 Dispatcher）
 +-------------------------------------------------------------------+
 |  Loop（补充）：为步骤排序                                         |
 |    模型请求 -> context/prepare（交接，不变）                      |
 |    压缩步骤（手动）-> ContextPolicy::compact_step                 |
 |  ContextPreparation（补充）                                       |
 |    准备中: ContextPolicy::prepare -> compact_step                 |
 |    （阈值、预算恢复）                                             |
 |  ContextPolicy::compact_step: K 轮切点、host.compact、验收、      |
 |    追加 context/compaction、快照（所有触发共用一条路径）          |
 +-------------------------------------------------------------------+
```

两个 Session 部件分开，是因为依赖不同：分支注入 `ContextModelHost`，它的提供者不能依赖由分支树提供的 `TreeCommands`。

### 2.1 手动步骤放在哪里

Loop 在一个受跟踪的任务中，调用 `ContextPolicy::compact_step` 自己运行手动压缩步骤，而不是用事件驱动 ContextPreparation。理由：

- **Loop 已经持有步骤需要的东西。** 它的 `Deps` 已经注入 `Timeline`、`HistoryRenderer` 和 `ContextModelHost`（[agent_loop.rs](../../../../crates/dim-agent-capabilities/src/agent_loop.rs) `LoopPlugin::Deps`）。它在 `start_preparation` 中本来就读取固定上下文，并由最新的 `context/configured` 构造请求。ContextPolicy 是同一 crate 的内部结构体。
- **没有东西要交接。** `context/prepare` 的作用是把请求交给 ContextPreparation，再交给 ModelProvider。手动步骤不发模型请求。为一个 ContextPreparation 并不排序的步骤做交接，只会多出第二条指令和第二个单飞槽（ContextPreparation 的 `current`）。
- **结构上不会竞争。** 压缩步骤的两个调用方不可能同时运行：
  - Loop 只在分支没有在途请求时启动手动步骤，步骤运行期间也不启动请求。
  - ContextPreparation 只在某个在途请求的准备中调用这个步骤。

  所以只有一个排序者（Loop）、一个压缩函数、一种事实。

## 3. 数据与状态

| 项 | 所有者 | 类别 |
| --- | --- | --- |
| `context/compact {}` | Loop | 新指令事件（§4.3） |
| `context/compaction` | 由 ContextPolicy 代其调用方（ContextPreparation 或 Loop）写入 | 现有事实，有改动：`turn_id`、`request_id` 变为可选，新增 `command` 和 `cancelled` 结果（§4.3） |
| 压缩槽：`{command, running: Option<CancellationToken>}` | Loop | 私有。由 `context/compact` 和 `context/compaction` 折叠，装载时重建（§5.4） |

宿主不持有状态。解析出的连接只存在于一次 `compact` 调用内。

## 4. 接口

### 4.1 Session 方法（语言中立，ADR 002 D5）

`sessionMethods`，Session 作用域，键 `session.context`，由 ContextMethods 提供（[session-boundary A.1](../contracts/session-boundary.md#a1-方法贡献)）。两个方法都声明为 `interruption: report`、`progress: false`、`notifications: []`、`reply: direct`。

| 方法 | 变更 | 请求 | 回复 |
| --- | --- | --- | --- |
| `session.context.compact` | 是 | `{sessionId, branchId}` | `{status: "completed", sequence}` 或 `{status: "skipped", reason}` |
| `session.context.usage` | 否 | `{sessionId, branchId}` | `{through, contextWindow, outputTokens, thresholdTokens, estimatedInputTokens, baselineSource: "provider_baseline" \| "full_estimate"}` |

`session.context.compact`：

- 经 `AgentLoop::compact` 写 `context/compact`，该调用返回指令的序号 `c`。
- 然后在分支 Timeline 上等待第一条 `command ≥ c` 的 `context/compaction`。合并到同一个步骤的多条指令（§5.3）由该步骤的事实一起服务。
- 结果映射：`completed` → `{status: "completed", sequence}`（事实的序号）；`skipped` → `{status: "skipped", reason}`；`failed` → `compaction_failed`；`cancelled` → `compaction_cancelled`。

`session.context.usage`：

- `through` 是计算这些数值时的已提交头部。
- 数值等于在该头部做一次准备时写进 `context/budget` 的值，外加 `thresholdTokens`。

取消按 session-boundary A.1：卸载会中断调用，调用方断开不取消任何东西。

### 4.2 ContextModel 端口（现有，Rust）

F1 给 `CompactionInput` 加两个字段，都由生产者 ContextPolicy 设置：

- `cut: String`：宿主摘要到哪一条为止（§5.2）。
- `trigger: CompactionTrigger`：`Threshold`、`ProviderOutputBudgetExhausted` 或 `Manual`。

### 4.3 事件

| 事件 | 写入方 | 消费方 | 数据 |
| --- | --- | --- | --- |
| `context/compact`（新） | Loop，经其公开写操作 `AgentLoop::compact` | Loop，按序号 | `{}` |
| `context/compaction`（有改动） | ContextPolicy，代其调用方 | Loop（手动步骤的终态）、ContextPolicy（重试复用）、投影、ContextMethods | `{turn_id?, request_id?, command?, through, outcome}` |

`context/compaction` 的字段规则：

| 来源 | `turn_id` | `request_id` | `command` |
| --- | --- | --- | --- |
| 自动（阈值、预算恢复） | 该请求所在的 turn | 该请求 | 无 |
| turn 内的手动步骤 | 该 turn | 无 | 被服务的 `context/compact` 的序号 |
| 没有 turn 时的手动步骤 | 无 | 无 | 被服务的 `context/compact` 的序号 |

- `outcome` 新增 `cancelled {reason}`，因此每个手动步骤都恰好以一条事实结束。
- 自动压缩被取消时的行为不变：不写事实，请求以 `Cancelled` 结束。
- 结构体直接修改，读取方随之更新：ContextPolicy 的重试过滤改为把 `turn_id` 与 `Some(turn)` 比较；Trace 的 id 提取改为可选。原生存储只按 `through` 索引已完成的压缩，不受影响。
- 冻结的 v1 wire 样本不改；v1 到原生的转换把其中必填的 id 映射为 `Some`。

**动态事件（决定 A）。** 现在先把 `context/compact` 作为 `EventKind` 变体加入。#2 落地后（插件注册自己的事件类型；Timeline 检查结构与类型范围，不检查内容），它迁移为由 Loop 拥有的动态注册事件。F1 是 #2 的第一个真实使用者。`tree_protocol.rs` 中穷举的 `ProductEvent` 分类把 `context/compact` 列为客户端不可见，与 `context/compaction` 相同。

### 4.4 Rust 绑定

```rust
// dim-agent-types/src/events.rs
EventKind::ContextCompact(ContextCompactData {})                  // "context/compact"
pub struct ContextCompactionData {
    pub turn_id: Option<String>, pub request_id: Option<String>,
    pub command: Option<u64>, pub through: u64, pub outcome: ContextCompactionResult }
pub enum ContextCompactionResult { Completed{..}, Skipped{..}, Failed{..},
                                   Cancelled { reason: String } }

// dim-agent-capabilities/src/host.rs（同一 crate 的现有端口）
pub struct CompactionInput { /* 现有 */ pub cut: String, pub trigger: CompactionTrigger }

// dim-agent-capabilities/src/context_policy.rs（crate 内部）
impl ContextPolicy {
    /// 唯一的压缩路径：切点、host.compact、验收、追加、快照。
    async fn compact_step(&self, step: CompactionStep, cancel: CancellationToken)
        -> Result<Arc<AgentEvent>, HostError>;   // 已提交的 context/compaction
}

// dim-agent-capabilities/src/agent_loop.rs（补充）
impl AgentLoop { pub fn compact(&self) -> Result<u64>; }  // 追加 context/compact
impl PublicService for AgentLoop {}                          // lib.rs

// dim-agent-capabilities/src/context_preparation.rs（补充）
impl ContextPreparation {
    pub async fn usage(&self, cancel: CancellationToken) -> Result<ContextUsage, ContextOpError>;
}
impl PublicService for ContextPreparation {}                // lib.rs

// dim-agent-capabilities/src/context_model.rs（新）
pub struct ProductContextModelPlugin { scope: SessionScope }
//   Deps = (Arc<ModelHost>, Arc<ProviderConnectionResolver>)  -- 进程作用域
// dim-agent-capabilities/src/context_methods.rs（新）
pub struct ContextMethodsPlugin { scope: SessionScope }
//   Deps = InSession<TreeCommands>；SessionMethods::provide(ctx, &scope, "session.context", ..)

// dim-app-server/src/main.rs，仅产品配置（没有 --models 文件）：
builder.session_plugin(ProductContextModelPlugin::new)
       .session_plugin(ContextMethodsPlugin::new)
       .session_methods("session.context");
// 在这里替换 ContextModelHost(None)；--models 与 mock 配置保持不变
```

## 5. 流程

### 5.1 生产宿主

`ProductContextResolver::resolve(request)`：

1. 要求 `request.capability_snapshot` 存在且对请求的模型有效。没有时返回 `InvalidRequest`“Model capabilities are not adopted; select the model again”，条件和文本与 `ProductResolver` 相同。
2. 返回绑定该请求的 `ContextModel`：

| 方法 | 值 |
| --- | --- |
| `context_window` | `snapshot.capabilities.contextWindow`，即目录值，含其现有覆盖 |
| `output_tokens` | 有 `request.config.maxTokens` 时取它，否则取 `snapshot.capabilities.maxOutputTokens`；不超过后者 |
| `threshold_tokens` | `0.8 × (context_window − output_tokens)`，暂定（#128） |
| `count_tokens` | 序列化后的系统提示、消息和工具声明的 UTF-8 字节数除以 4；每个图片部件按固定 1,600 token 计。暂定（#128）。用于：在提供方的 `prompt_tokens` 之上算增量、第一个请求、`validate`、以及检查压缩是否减少了输入 |
| `compact` | 见下 |

`compact(input, cancel)` 摘要上一份摘要（`input.checkpoint`）加上直到并包括 `input.cut` 的历史条目，返回 `Completed {cursor: input.cut, summary, diagnostics}`：

- **请求。** 每块一个摘要请求。用请求的 provider、model、快照和 config，`maxTokens = output_tokens`，系统提示是本模块拥有的固定摘要提示，一条包含记录文本的 user 消息，不带工具。
- **凭据与失败。** 连接来自 `ProviderConnectionResolver`，只在本次调用内使用。调用经 `invoke_with_recovery` 运行，即 ModelProvider 的失败与凭据策略。S4 抽出了它；如果 F1 先合入，由 F1 做同样的抽取。
- **输出。** 丢弃 thinking 输出。工具停止或空文本算失败。
- **记录文本。** 按角色标注的文本：工具调用写名称和输入，工具结果写文本，图片写 `[image]`。
- **分块。** 单条超过一次调用输入预算的，从中间截断，计入 `compactedToolPayloadCount`。更长的记录分块折叠：每次调用拿到当前摘要和下一块。
- **诊断。** 必填的诊断字段来自本次运行（`reason` 和 `trigger` 取自 `input.trigger`）。

### 5.2 一条压缩路径（ContextPolicy）

`compact_step` 就是把 `ContextPolicy::prepare` 中现有的压缩部分抽出来：

1. 选切点。
2. 调用 `host.compact`。
3. 验收切点，并确认压缩后的请求更小且低于窗口。
4. 带上调用方的 id 和 `command`，追加 `context/compaction`。
5. 保存可选快照。

调用方：

- **ContextPreparation，在 `prepare` 内。** 估算达到阈值时（与现在一样，每个边界一次），或在 `recovery` 下。触发原因为 `Threshold` 或 `ProviderOutputBudgetExhausted`。`prepare` 的其余部分不变：估算、`context/budget`、重试时复用 `through` 和 turn 相同的压缩、在已提交的事实上投影请求。
- **Loop 的手动步骤。** 触发原因为 `Manual`。用于 `resolve` 和估算的请求按 `start_preparation` 的方式构造：最新 `context/configured` 的设置和 system，加上投影后的消息。不绑定工具。

**切点**对所有触发都由 ContextPolicy 选择：

- 保留的是分支上的最后 K 轮，含正在进行的那一轮，一轮从 `turn/start` 到 `turn/end`。
- 条目按其来源事件在固定上下文中的序号归属到某一轮。从快照状态恢复的条目早于该快照的压缩，算作比任何保留轮都早。
- 切点是最老保留轮的第一条之前最晚的合法切点。合法切点指现有检查接受的切点：不保留被移除调用的工具结果，下一条也不是工具上下文。
- 没有这样的切点时：在 `recovery` 下退回到最晚的合法切点，让请求仍然放得下；否则结果是 `skipped {reason: "nothing_to_compact"}`，不调模型。

### 5.3 手动压缩

**在 turn 内：**

```
      step/model-start {T, R1} ... step/model-output-end {T, R1, completed}
 k    context/compact {}                          写入方：Loop（AgentLoop::compact）
      -- Loop 消费 k：有在途请求 -> 待处理
      step/tool-call {T, R1} ... step/tool-result {T, R1}
      -- Loop 消费工具结果（边界 b）：待处理 -> 压缩步骤，暂不开始下一个请求
 m    context/compaction {turn_id: T, command: k,  写入方：ContextPolicy（Loop 的步骤）
                          through: b, outcome}
      -- Loop 消费 m：步骤结束 -> 照常 next_model
      input/consumed* ; step/model-start {T, R2}   Loop（不变）
      context/prepare {T, R2, ...} -> model/call   基于压缩后的上下文
```

**没有 turn 在运行：**

```
 k    context/compact {}                          写入方：Loop（AgentLoop::compact）
      -- Loop 消费 k：没有在途请求 -> 立即运行压缩步骤
 m    context/compaction {command: k, through: k, outcome}   写入方：ContextPolicy
      -- Loop 消费 m：步骤结束 -> try_start
      （没有 turn/start，没有 turn/end；步骤期间应用的驱动输入此时启动 turn，
       其第一个请求基于压缩后的上下文准备）
```

**调度规则：**

- **边界。** 步骤边界是 Loop 本要准备下一个请求的地方（不带预算重试 source 的 `next_model`），或者 turn 结束的地方。待处理的步骤在第一个边界启动。
  - turn 结束而不是继续时，步骤在 `turn/end` 之后运行，不属于任何 turn。
  - 预算重试属于它的请求，不算边界。
- **through。** `through` 是触发步骤启动的那个被消费事件的序号：没有 turn 时就是 `context/compact` 本身，turn 内是边界事件。
- **合并。** 步骤处于待处理时消费到的指令合并进它，事实引用最后合并的那条指令。步骤运行期间消费到的指令，在它之后再安排一个步骤。
- **不推进。** 步骤运行期间 Loop 不推进：`next_model` 和 `try_start` 等待步骤的事实。输入照常排队，不被步骤消费，因此永远不会落入被摘要的范围。

**steer、中断与重载：**

- **steer。** 步骤期间应用的驱动输入不取消步骤。在 turn 内，下一个请求消费该输入；没有 turn 时，该输入在步骤结束后启动一个 turn。
- **中断。** `stop/interrupt` 取消正在运行的步骤，丢弃待处理的步骤。这与自动压缩一致：
  - 宿主已经返回的压缩照样保留，作为 `completed` 提交。
  - 否则取消宿主调用，事实为 `cancelled {reason}`。
  - 被丢弃的待处理步骤立即写 `cancelled`。
  - 在 turn 内，随后经现有的 `next_model` 检查，turn 以 `Cancelled` 结束。
- **重载。** 已消费但没有对应事实的 `context/compact` 当时在途。新一代 Loop 写 `context/compaction {command, cancelled: "capability reloading"}`，不重发摘要调用。被重建的 turn 照旧以 `Failed` 结束。

### 5.4 Loop 新增的内容

全部在 `crates/dim-agent-capabilities/src/agent_loop.rs`：

1. **订阅。** 在现有列表中加入 `context/compact` 和 `context/compaction`。
2. **公开写操作。** `AgentLoop::compact()` 追加 `context/compact` 并返回其序号。`AgentLoop` 加上 `PublicService` 标记。
3. **压缩槽**（`command` 和运行令牌），由已消费事件折叠。消费 `context/compact` 时，Loop 把步骤标为待处理；没有在途请求时立即启动步骤。
4. **边界。**
   - `next_model`（带预算重试 source 时除外）和 `turn/end` 的处理器有待处理步骤时先启动步骤，而不是推进。
   - 步骤运行期间，`next_model` 和 `try_start` 什么都不做。
5. **步骤任务。** 与准备任务一样受跟踪。它按 `start_preparation` 的方式构造请求，然后调用 `ContextPolicy::compact_step`（触发原因 `Manual`；有 turn 时带 turn id；带 `command`）。处理器只登记任务就返回。
6. **步骤终态。** 消费到带 `command` 的 `context/compaction` 时，Loop 清空压缩槽并继续：在 turn 内调用 `next_model`，否则调用 `try_start`。
7. **中断。** `stop/interrupt` 取消运行令牌，并为待处理的步骤写 `cancelled`。steer 不影响步骤。
8. **重建。** 压缩槽由事件折叠；在途的指令以一条 `cancelled` 事实结算。

不变的部分：turn 与请求的生命周期、输入消费、工具批次、对模型请求的 steer、预算恢复、`LoopSnapshot` 和 `context/prepare`。

### 5.5 ContextPreparation 新增的内容

- `ContextPolicy::prepare` 改为调用抽出来的 `compact_step`，带 K 轮切点和触发原因。其准备、steer、中断和重载处理都不变。
- `usage(cancel)` 读取已提交头部，按 §5.2 构造请求，运行现有估算。不写任何东西，可以在 turn 进行中调用。

## 6. 错误

`MethodError` 代码（未注明的为 `code -32000`）：

| 代码 | 方法 | 何时 | 重试 |
| --- | --- | --- | --- |
| `-32602` | 两者 | 缺少 `branchId` 或不是字符串 | 否 |
| `session_invalid_input` | 两者 | 未知分支（`AgentError::UnknownBranch` 的现有映射） | 否 |
| `session_closed` | 两者 | Session 或分支正在关闭 | 否 |
| `context_not_configured` | usage | 分支历史中没有 `context/configured` | 采用模型之后 |
| `context_model_failed` | usage | `resolve` 失败（没有能力快照） | 重新选择模型之后 |
| `compaction_failed` | compact | 事实为 `failed`：共享失败策略之后的宿主错误、空摘要、非法切点、没有减少输入、没有 `context/configured`，或没有能力快照 | 由用户决定 |
| `compaction_cancelled` | compact | 事实为 `cancelled`：`stop/interrupt` 或重载 | 由用户决定 |
| `operation_interrupted` | compact | 等待期间 ContextMethods 卸载（session-boundary A.1，`report`）；指令仍在 Timeline 中，照样会被服务或结算 | 由用户决定 |

自动压缩的错误处理不变。宿主错误成为 `failed` 事实，请求不压缩继续；在恢复中则使请求失败。

## 7. 安全

- **凭据只在请求内。** 连接在一次 `compact` 调用内从 `ProviderConnectionResolver` 取得并交给 `ModelHost`。它从不被保存、返回、序列化或记录。Provider 不变。
- **没有新的数据外发。** 摘要调用把 Session 自己的历史发给每个 turn 都会收到它的提供方和模型。
- **摘要是不可信的模型输出。** 它只经现有的 `summary_message` 进入上下文，被标为数据并转义。
- **结果中没有密钥。** 事实、回复和诊断只含计数、id 和 `HostError` 文本，该文本不含凭据值。请求和摘要内容不进 tracing。
- **授权。** Rpc 对 Session 方法校验 `sessionId`（C1）。能调用 Session 方法的连接都可以压缩，正如它可以提交输入或停止。

## 8. 与现有部件的关系

| 部件 | 关系 |
| --- | --- |
| Loop | 补充（§5.4）：压缩步骤。产品配置下 `ContextModelHost` 为 `Some`，预算恢复生效 |
| ContextPreparation、ContextPolicy | 补充（§5.2、§5.5）：抽出 `compact_step`、K 轮切点、触发原因、`usage` |
| ModelProvider | 代码不变。产品配置下 `validate` 开始按估算检查窗口 |
| Provider、ProductResolver、Aimux | 不变；经 `ProviderConnectionResolver` 和 `ModelHost` 使用 |
| AuxiliaryModel（S4） | 不使用。共用 `invoke_with_recovery`；谁先合入谁负责抽取 |
| `sessionMethods`（C1） | 两个方法需要它。宿主部件不需要 C1，可以先合入 |
| turn 的消费方（客户端、将来的 Goal、Hooks） | 不受影响。turn 之外的手动压缩不写 `turn/*`；turn 内的手动压缩只是该 turn 的一个步骤 |
| #2 动态事件 | #2 落地后 `context/compact` 迁移为动态注册；F1 是它的第一个真实使用者 |
| Trace、HistoryRenderer、快照 | 投影不变；手动事实与自动事实一样被投影和做快照；Trace 读取可选 id |
| `--models` 配置、mock 配置 | 保持 `ContextModelHost(None)`：不压缩，也不登记 `session.context` |
| ManagedProviders 套餐用量、CLI `/compact` 与 `/usage` | 不在 F1 的验收内；CLI 调用这两个方法 |

## 9. 规则检查

| 规则 | 回答 |
| --- | --- |
| 原则 §1（Capability、App Server 边界） | 两个 Session Capability 和两处补充都在 `dim-agent-capabilities`；App Server 只在产品配置中安装它们。 |
| §1 唯一事件总线 | 指令和事实都在分支 Timeline 上，经其 Dispatcher 消费。方法的等待读取的也是这个 Timeline。没有旁路。 |
| §3 依赖方向 | 新部件依赖核心服务（`ModelHost`、`ProviderConnectionResolver`、`TreeCommands`、Loop、ContextPreparation）。现有部件只经现有的 `ContextModelHost` 键用到它们，而安装该部件的地方总会提供这个键。没有可选依赖。 |
| §3 改动其他模块（Loop） | 已批准的改动。需求来自 Loop 自己对上下文窗口的控制：压缩是 Loop 的一个步骤，和模型调用、工具批次一样，Loop 不能在一个将被摘要取代的上下文上推进。它在 Loop 内闭环：自动压缩本来就在 Loop 的请求准备中运行，手动步骤在同一个排序者之下使用同一个函数。没有其他模块因此改变形状。 |
| §4 规则 1（指令事件） | 客户端用 `context/compact` 请求推进过程的所有者。Loop 决定何时服务它，结果是一条引用该指令的 `context/compaction` 事实。 |
| §4 规则 2（交接） | 模型请求仍以 `context/prepare` 交接。手动步骤不交接任何东西。 |
| §4 规则 3（before 回调；处理器） | 不用 before 回调。Loop 的处理器只登记步骤任务就返回。 |
| §4 规则 4（核心无业务含义） | Timeline、Dispatcher 和门面不变。 |
| §4 规则 5（原子批次） | 每次新写入都是一个事件。 |
| §4 规则 6（上下文只来自交接后的事件） | 摘要只经已完成事实的现有投影到达模型。待处理输入不被步骤消费（AGENTS 规则 10）。 |
| §4 规则 7（公开事件与服务） | `TreeCommands`、`PublicService`（`AgentLoop`、`ContextPreparation`）上的 `Branch::service`、`ProviderConnectionResolver`、`ModelHost`、`sessionMethods`。 |
| §4 规则 8（基础设施不变） | Timeline、Dispatcher、信封、rutis、loader、Agent 装配和门面都不改。新事件类型是现有信封中的插件内容，#2 落地后迁移为动态注册。不向 rutis 提需求。 |
| §4 规则 9（只写需要的事实） | `context/compact` 由 Loop 后续的决定和重载读取。`context/compaction` 用于投影，并作为每个手动步骤的结束。用量查询不写任何东西。 |
| ADR 002 D1（作用域） | 宿主与方法：Session。指令、步骤和压缩槽：分支，在 Loop 中。 |
| ADR 002 D2（无接线模块） | ContextMethods 经规定的 `sessionMethods` 暴露所有者的操作；ProductContextModel 是本功能的宿主。 |
| ADR 002 D3（依赖） | 只硬依赖核心。Provider 或 Aimux 重载会像传播到 ModelProvider 一样传播到这些部件（已接受的代价）。 |
| ADR 002 D4（配置） | 不增加配置；暂定常量写在代码里（#128）。 |
| ADR 002 D5（语言中立合同） | §4.1 与 §4.3 是合同；§4.4 是一种绑定。 |

**影响面（公开内容）：**

- `session.context` 命名空间（`compact`、`usage`）。
- 指令事件 `context/compact`。
- 改动后的 `context/compaction`（可选 id、`command`、`cancelled`）。

内部 Rust 改动：`CompactionInput.cut` 与 `.trigger`；`AgentLoop` 与 `ContextPreparation` 上的 `PublicService`。

**向机制所有者提出的需求：** 无。已在计划中的 #2 将以动态事件承载 `context/compact`。

## 10. 决定与待定问题

| 项 | 决定 |
| --- | --- |
| 用哪个模型压缩 | 请求自己的模型；单独的压缩模型推迟 |
| 触发点与保留量 | `0.8 × (窗口 − 输出)`、K = 2 轮、`字节数 / 4`、每张图片 1,600 token：暂定，在 #128 研究 |
| 手动压缩 | 由 `context/compact` 指令安排的 Loop 步骤，在 turn 内外都可以，永远不创建 turn |
| 步骤在哪里运行 | 在 Loop 中，调用 ContextPolicy 唯一的压缩函数（§2.1） |
| `context/compaction` | 直接修改：可选 id、`command`、`cancelled`；软件尚未发布，没有存储版本问题 |
| 新事件类型 | 现在先作为 `EventKind` 变体，#2 之后改为动态注册 |
| `--models` 配置 | 没有能力快照，不压缩：接受 |
| 提供方报告的溢出 | 单独的 Aimux issue，不在 F1 内 |

待定：除 #128 外没有。

## 11. 验收（流程测试）

测试用产品配置的 App Server 连接一个上下文窗口很小的 mock HTTP 提供方（`crates/dim-app-server/tests`），另加 `crates/dim-agent-capabilities/tests` 中的 Loop 流程测试。

1. **自动。** 超过阈值后，下一次准备写 `context/budget`，并发出一个摘要请求（摘要提示、一条 user 消息、不带工具、Session 的模型）。它提交 `completed` 的 `context/compaction`：带该请求的 id，没有 `command`，`trigger: threshold`，切点在最后 2 轮之前。模型请求以摘要开头，原样保留最后 2 轮。
2. **预算恢复。** 估算达到窗口的请求以预算事实结束。Loop 重试一次，重试时以 `trigger: provider_output_budget_exhausted` 压缩。
3. **手动，没有 turn。** `session.context.compact` 只写 `context/compact`，然后是 `context/compaction {command}`；没有 `turn/*`、没有 `step/*`、没有 `model/call`。回复为 `completed` 并带事实的序号。步骤期间提交的输入在事实之后启动 turn，其请求以摘要开头。
4. **手动，turn 内。** 指令在工具批次期间写入，序列为：`step/tool-result` → `context/compaction {turn_id, command}` → 下一个请求的 `step/model-start`。该请求基于压缩后的上下文准备，turn 继续。
5. **中断与重载。**
   - 步骤期间 `session.stop` 写 `cancelled`（宿主已经返回时为 `completed`），回复与之一致。
   - 步骤期间重启写带重载原因的 `cancelled`，不发第二次摘要请求。
6. **用量。** `session.context.usage` 返回目录窗口、输出预算、阈值和估算。压缩之后估算下降，不写任何事件。
7. **失败与凭据。** 摘要请求失败时事实为 `failed`：自动路径不压缩继续，手动回复 `compaction_failed`。事实、回复和诊断中不出现 API key。
