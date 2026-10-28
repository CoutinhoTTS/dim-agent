# ContextCompaction

状态：已在 F1 分支实现（设计已批准；实现说明见 §12）。路线图项 F1（[#72](https://github.com/arcships/dim/issues/72)）。
总体设计：[附录 B.1 ContextPreparation 行](../overview.md#b1-session-与分支-capability)。
作用域：[ADR 002](../../adr/002-plugin-scope.md)：若干 Session 部件，加上对现有分支 Capability Loop 与 ContextPreparation 的补充。§12 记录实现对本文的细化。触发点、保留与恢复规则遵循 [#128](https://github.com/arcships/dim/issues/128) 的研究（[compaction-trigger](../research/compaction-trigger.md)）；其中的余量和保留份额是暂定的经验值。§13 是摘要评估、摘要器推理与目标长度的已批准设计（[#170](https://github.com/arcships/dim/issues/170)）；默认值由运行该评估决定（§13.4）。英文版：[ContextCompaction](../../../design/plugins/ContextCompaction.md)。

## 1. 概要

**压缩**是用一段更短的替换内容取代切点之前的历史，并原样保留最后几轮。被替换的范围通常是上一个压缩点以来的全部内容，加上上一份替换内容。此后分支在新结果上推进。

压缩是 Loop 控制上下文窗口的一部分。它是 Loop 的一个步骤，和模型调用、工具批次一样，不绑定 turn。压缩步骤运行期间，Loop 不推进到下一步。否则压缩期间上下文可能溢出，新事件也会基于旧上下文决定，与替换内容不一致。

自动压缩已经在请求的准备中运行（`context/prepare` → 压缩 → `model/call`），Loop 等待该请求的终态（[context_policy.rs](../../../../crates/dim-agent-capabilities/src/context_policy.rs)）。但它在生产环境里从不运行，因为 App Server 绑定的是 `ContextModelHost(None)`（[main.rs](../../../../crates/dim-app-server/src/main.rs)）。

F1 把压缩分成三层（§2.2），并新增：

1. **模型窗口事实。** `ContextModel` 现在只携带窗口事实：上下文窗口、输出预算、阈值和 token 计数。`ProductContextModel` 从请求上已采用的能力快照中取得这些事实，token 计数用本地估算。
2. **策略端口。** 新端口 `CompactionStrategy` 负责产出替换内容。`ProductSummaryStrategy` 是 F1 唯一实现的策略：它用请求自己的模型做摘要，经现有的 `ProviderConnectionResolver` 和 `ModelHost` 调用模型。
3. **通用记录。** 已完成的 `context/compaction` 携带 `{retain_from, replacement: Message[], strategy, …}`，其中 `retain_from` 是第一条原样保留的 Timeline 事件的 id。所有策略共用一条重放规则：`retain_from` 之前的全部内容由 `replacement` 取代。
4. **一条压缩路径。** ContextPolicy 的压缩步骤是一个函数。自动压缩在准备中调用它，Loop 的手动步骤也调用它。
5. **手动压缩是 Loop 的一个步骤。** `session.context.compact` 写指令 `context/compact`，由 Loop 安排一个压缩步骤：
   - 在 turn 内，步骤在下一个步骤边界运行，即准备下一个请求之前。
   - 没有 turn 在运行时，步骤立即运行，只写它自己的压缩事实。
6. **压缩状态是事实。** 每次策略调用都由两条 Timeline 事实包住：调用前写的 `context/compaction-start`，以及恰好一条引用它的 `context/compaction` 结果。客户端、历史和重载结算都读取这两条事实（§4.3、§5.7）。
7. **用量查询。** `session.context.usage` 返回当前预算，不写任何东西。

不变的部分：Provider、Aimux、ModelProvider、Timeline、Dispatcher、事件信封、rutis 和 `context/prepare`。F1 新增事件类型 `context/compact` 与 `context/compaction-start`，并直接修改 `context/compaction`；软件尚未发布，接受破坏性修改。

### 1.1 为什么这么小

- **不用 AuxiliaryModel。** S4 的 `auxiliaryModel.complete` 只返回文本。压缩需要被压缩上下文所属模型的窗口事实（[auxiliary-model §1.1](https://github.com/arcships/dim/blob/roadmap/s4-provider-purpose-policy-and-auxiliarymod/docs/design/appserver/auxiliary-model.md)）。
- **不放进 Provider。** 能力快照随 `ModelRequest` 传递；凭据由 `ProviderConnectionResolver` 解析，与 ModelProvider 每个 turn 的做法相同；执行用 `ModelHost`。
- **没有第二个状态机。** 所有步骤由 Loop 排序。ContextPreparation 只在 Loop 已经交给它的请求准备中运行压缩函数。

## 2. 结构

```
 进程作用域（现有，不变）
 +-------------------------------------------------------------------+
 |  Aimux: ModelHost        Provider: ProviderConnectionResolver     |
 +----------------------------------^-------------------^------------+
                                    | 注入（硬依赖）    |
 Session 作用域                     |                   |
 +----------------------------------+-------------------+------------+
 |  ContextModelHost(Some(ProductContextModel))   Session 值（新）   |
 |  ProductSummaryStrategy（新部件）                                 |
 |    提供 S:CompactionStrategyHost(Some(摘要策略))                  |
 |  ContextMethods（新部件）                                         |
 |    把 session.context 注册到 sessionMethods：compact, usage       |
 |      compact -> TreeCommands.get(b).service::<AgentLoop>().compact()
 |      usage   -> TreeCommands.get(b).service::<ContextPreparation>().usage()
 +---------------------------------^---------------------------------+
            Rpc（C1）分发 ---------'
 分支作用域（一个 Timeline，一个 Dispatcher）
 +-------------------------------------------------------------------+
 |  Loop（补充）：为步骤排序                                         |
 |    模型请求 -> context/prepare（交接，不变）                      |
 |    压缩步骤（手动）-> ContextPolicy::compact_step                 |
 |  ContextPreparation（补充）                                       |
 |    准备中: ContextPolicy::prepare -> compact_step                 |
 |  ContextPolicy::compact_step: 保留规则、strategy.compact、验收、  |
 |    追加 context/compaction、快照                                  |
 |  投影（history.rs）：replacement + 基线 + 从 retain_from 起的条目  |
 +-------------------------------------------------------------------+
```

`ProductContextModel` 不需要任何服务，所以是一个普通的 Session 值。ContextMethods 与策略部件分开：分支注入 `CompactionStrategyHost`，它的提供者不能依赖由分支树提供的 `TreeCommands`。

### 2.1 手动步骤放在哪里

Loop 在一个受跟踪的任务中，调用 `ContextPolicy::compact_step` 自己运行手动压缩步骤，而不是用事件驱动 ContextPreparation。理由：

- **Loop 已经持有步骤需要的东西。** 它的 `Deps` 已经注入 `Timeline`、`HistoryRenderer` 和 `ContextModelHost`（[agent_loop.rs](../../../../crates/dim-agent-capabilities/src/agent_loop.rs) `LoopPlugin::Deps`），F1 再在旁边加上 `CompactionStrategyHost`。Loop 在 `start_preparation` 中本来就由最新的 `context/configured` 构造请求。
- **没有东西要交接。** `context/prepare` 把请求交给 ContextPreparation，再交给 ModelProvider。手动步骤不发模型请求；做交接只会多出第二条指令和第二个单飞槽（ContextPreparation 的 `current`）。
- **结构上不会竞争。** Loop 只在分支没有在途请求时启动手动步骤，步骤运行期间也不启动请求。ContextPreparation 只在某个在途请求的准备中调用这个步骤。所以只有一个排序者、一个函数、一种事实。

### 2.2 扩展点

| 层 | 固定什么 | 在哪里 | 新增策略时是否改动 |
| --- | --- | --- | --- |
| 机制 | 压缩何时运行，以及运行期间 Loop 停住：准备中的自动触发、Loop 的手动步骤、保留规则、验收、提交 | Loop、ContextPolicy | 否 |
| 合同 | 记录及其重放：`context/compaction {retain_from, replacement, strategy, context_baseline}`，`retain_from` 是 Timeline 事件 id；重放依次放 `replacement`、按键的基线、由 `retain_from` 及其后事件放置的条目。记录引用的是 Timeline 事件而不是渲染细节，所以换一个 `HistoryRenderer` 或策略仍能找到切点。重放是确定的，所以 fork、重载和提示缓存前缀看到的消息相同 | `dim-agent-types`、`history.rs` 投影 | 否 |
| 策略 | 替换内容是什么，以及在保留规则之内选哪个切点 | Session 值 `CompactionStrategyHost` | 被替换 |

**安装另一种策略。** 替换 `CompactionStrategyHost` 这个 Session 值即可：装配注册另一个值，或注册另一个提供它的 Session 部件，代替 `ProductSummaryStrategy`。

- ContextPreparation 和 Loop 依赖的是这个键，不是策略插件，所以不新增可选依赖。
- 旧记录照样能重放，因为重放从不调用策略。
- 以后的策略举例：滑动窗口加笔记，或把旧的工具调用渲染成简短占位。

**F1 不做：**摘要以外的其他策略，以及多种策略的组合。

## 3. 数据与状态

| 项 | 所有者 | 类别 |
| --- | --- | --- |
| `context/compact {}` | Loop | 新指令事件（§4.3） |
| `context/compaction-start` | 由 ContextPolicy 代其调用方写入 | 新事实（§4.3） |
| `context/compaction` | 由 ContextPolicy 代其调用方写入；重载时由负责结算的所有者写入 | 现有事实，有改动（§4.3） |
| 压缩槽：至多一条未完成指令 `{sequence, consumed, running: Option<CancellationToken>}` | Loop | 私有。由 `AgentLoop::compact` 或消费指令时设置，由该指令的结果清除。装载时结算（§5.5） |

Session 值不持有状态。解析出的连接只存在于一次策略调用内。

## 4. 接口

### 4.1 Session 方法（语言中立，ADR 002 D5）

命名空间 `session.context`，由 ContextMethods 注册到本 Session 的 `sessionMethods`（[session-boundary A.1](../contracts/session-boundary.md#a1-方法贡献)）。两个方法都声明为 `interruption: report`、`progress: false`、`notifications: []`、`reply: direct`。

| 方法 | 变更 | 请求 | 回复 |
| --- | --- | --- | --- |
| `session.context.compact` | 是 | `{sessionId, branchId}` | `{status: "completed", sequence}` 或 `{status: "skipped", reason}` |
| `session.context.usage` | 否 | `{sessionId, branchId}` | `{through, contextWindow, outputTokens, thresholdTokens, estimatedInputTokens, baselineSource: "provider_baseline" \| "full_estimate"}` |

`session.context.compact`：

- 调用 `AgentLoop::compact`，它返回指令序号 `c`：新写入的 `context/compact`，或该分支上已经未完成的那条指令（§5.4）。因此并发或重复的调用会加入同一个进行中的压缩。
- 然后在分支 Timeline 上等待带其指令的结果：第一条 `command = c` 的 `context/compaction`。
- 结果映射：`completed` → `{status: "completed", sequence}`；`skipped` → `{status: "skipped", reason}`；`failed` → `compaction_failed`；`cancelled` → `compaction_cancelled`。

`session.context.usage` 返回在已提交头部 `through` 处做一次准备时会写进 `context/budget` 的值，外加 `thresholdTokens`。

取消按 session-boundary A.1：卸载会中断调用，调用方断开不取消任何东西。

### 4.2 端口（Rust，运行时内部）

现有的 `ContextModel` 端口拆成两个，各自来自自己的 Session 值。

| 端口 | Session 值 | 方法 | F1 实现 |
| --- | --- | --- | --- |
| `ContextModel`（收窄） | `ContextModelHost(Option<Arc<dyn ResolveContextModel>>)`（现有） | `context_window`、`output_tokens`、`threshold_tokens`、`count_tokens`；`compact` 移出 | `ProductContextModel`（§5.1） |
| `CompactionStrategy`（新） | `CompactionStrategyHost(Option<Arc<dyn CompactionStrategy>>)` | `compact(input, cancel) -> Completed {retain_from, replacement, strategy, diagnostics} \| Skipped {reason, diagnostics} \| Err(HostError)` | `ProductSummaryStrategy`（§5.2） |

两个值在每种配置中都显式绑定，与现在的 `ContextModelHost` 一样。两者都是 `Some` 时策略存在。只有一个是 `Some` 属于装配缺陷，ContextPreparation 装载时报告。

`CompactionInput` 由 ContextPolicy 产生：

| 字段 | 含义 |
| --- | --- |
| `history` | 从上一个 `retain_from` 起的投影条目（现有） |
| `checkpoint` | 上一次的 `{retain_from, replacement, context_baseline}`（由 `cursor` 与 `summary` 改来） |
| `request`、`model` | 被压缩上下文所属的请求，及其 `ContextModel` 窗口事实（新） |
| `retain_limit` | 保留尾部放得进保留预算的最早合法切点的 `retain_from`（手动步骤至少为最新一轮的开头）；策略的 `retain_from` 必须在它或它之前（新；§5.3） |
| `trigger` | `Threshold`、`ProviderOutputBudgetExhausted` 或 `Manual`（新） |
| 各估算值、`provider_budget` | 现有 |

### 4.3 事件

| 事件 | 写入方 | 消费方 | 数据 |
| --- | --- | --- | --- |
| `context/compact`（新） | Loop，经其公开写操作 `AgentLoop::compact` | Loop，按序号 | `{}` |
| `context/compaction-start`（新） | ContextPolicy 在 `compact_step` 中代其调用方（Loop 的步骤或 ContextPreparation）写入 | Loop 与 ContextPreparation（重建）、客户端 | `{turn_id?, request_id?, command?, through, trigger: "manual" \| "threshold" \| "provider_output_budget_exhausted", strategy}` |
| `context/compaction`（有改动） | ContextPolicy，代其调用方；重载时由负责结算的所有者写入 | Loop（手动步骤的终态）、ContextPolicy（重试复用）、投影、ContextMethods、客户端 | `{turn_id?, request_id?, command?, start?, through, outcome}` |

`outcome` 为以下之一：

- `completed {retain_from, replacement: Message[], strategy: string, diagnostics, context_baseline}`。`retain_from`（Timeline 事件 id）取代渲染层的 `cursor`，`replacement` 取代 `summary: String`，`strategy` 标明产出方，例如 `"summary"`。
- `skipped {reason, diagnostics}`。
- `failed {error, diagnostics?}`。
- `cancelled {reason}`（新），因此每个手动步骤都恰好以一条事实结束。

| 来源 | `turn_id` | `request_id` | `command` |
| --- | --- | --- | --- |
| 自动（阈值、预算恢复） | 该请求所在的 turn | 该请求 | 无 |
| turn 内的手动步骤 | 该 turn | 无 | 被服务的 `context/compact` 的序号 |
| 没有 turn 时的手动步骤 | 无 | 无 | 被服务的 `context/compact` 的序号 |

**开始与结果的配对。**

- `compact_step` 在调用 `strategy.compact` 之前立即写 `context/compaction-start`。在调用任何策略之前就结束的压缩不写开始。
- 每条开始之后恰好有一条 `context/compaction`，其 `start` 等于该开始的序号，`turn_id`、`request_id`、`command`、`through` 与开始相同；结果为 `completed`、`skipped`、`failed` 或 `cancelled`。
- 没有 `start` 的结果只出现在一种情况：手动指令在调用任何策略之前就结束了。
  - 无法构造请求时为 `failed`（没有 `context/configured`、没有能力快照）；
  - 待处理的指令被中断丢弃或在重载时结算时为 `cancelled`。
- 没有走到策略调用的自动压缩两条事实都不写，与现在一样。
- 每个分支任何时候最多只有一条开始没有结果：Loop 不在有在途请求时运行手动步骤，ContextPreparation 一次只运行一个准备。
- 重载时，没有结果的开始意味着当时有压缩在途，由它的所有者写 `cancelled` 结果（§5.8）。自动压缩在开始之后被取消（中断或 steer）时，现在也写 `cancelled` 结果，而不再不写事实。

**切点：`retain_from`。** 已完成的结果指明第一条原样保留的 Timeline 事件。切点只落在事件边界上。

- **被替换的范围。** 从上一次已完成压缩的 `retain_from`（含；或从分支开头）到本次 `retain_from`（不含）之间的事件，再加上上一份 `replacement`。读者只凭两条记录就能推出这个范围。
- **合法性。** 在 `retain_from` 之前开始的工具调用，其结果不能在 `retain_from` 或之后。按键的基线照旧是 `retain_from` 之前那一刻的权威有效状态。
- **为什么用事件 id 而不是原来的游标。**
  - 原来的 `cursor` 是 `HistoryEntry` 的 id，例如 `<event id>:block:<i>`，依赖可注入的 `HistoryRenderer` 的 id 方案。换一个渲染器，旧记录就会失效。
  - 记录是用户和其他状态机从历史中读取的内容，所以必须引用 Timeline 事件，而不是渲染细节。事件 id 在 fork 继承前缀时也保持稳定。
  - 有了 K 轮保留，切点自然就是最早保留轮的第一个事件，本身就是事件边界。
- **投影时的映射。** ContextPolicy 和 `history.rs` 把 `retain_from` 映射到当前渲染中的位置：按序号，第一条来源事件在 `retain_from` 或之后的渲染条目。为此 `HistoryEntry` 携带放置它的事件的 id（`source`），条目自身的 id 方案因此不再重要。

**重放规则（所有策略）：**

- 投影出的消息依次是 `replacement`、按键的 `context_baseline` 块、由 `retain_from` 及其后事件放置的历史条目。
- `replacement` 按产出时的原样保存，所以重放从不调用策略，也不会重新渲染包装文本。
- 这会改动 `context_messages` 和 ContextPolicy 的 `project`（两者现在都放入 `summary_message(summary)`），以及 `CompactionCheckpoint`（用 `replacement` 代替 `summary`）。
- 历史快照的状态保存 `retain_from`，代替原来以条目 id 表示的 `compacted_through`；替换内容从锚定它的事实中读取。快照解释器版本随之升级。旧快照被绕过，这没有问题，因为快照只是缓存。
- 历史一侧的切点检查（`validate_cut` 和投影中的游标查找）改为比较事件 id，而不是条目 id。快照锚点由快照的所有者检查，而不是由 Timeline 检查（见下）。

**不含业务内容的快照锚定（#130）。** 目前 Timeline 核心和两个存储为了锚定投影快照，都会读取 `context/compaction` 的内容：

- `Timeline::context_with_checkpoint`（[timeline.rs](../../../../crates/dim-agent-runtime/src/core/timeline.rs)）要求锚点是 `EventKind::ContextCompaction`，比较 `data.through`，并把 `Completed { cursor }` 与 `checkpoint.state.compacted_through` 比对。
- `MemoryTimelineStore::save_snapshot`（[memory_timeline_store.rs](../../../../crates/dim-agent-runtime/src/memory_timeline_store.rs)）拒绝锚点不是已完成压缩的快照。
- SQLite 存储（[storage/timeline.rs](../../../../crates/dim-app-server/src/storage/timeline.rs)）保存时做同样的检查，查找 SQL 还按 `e.kind='context/compaction'` 和 `status='completed'` 过滤。

这违反了“Timeline 只检查信封与核心保留领域、不检查内容”的原则（[ADR 001](../../../adr/001-harness-foundation.md#events-and-ownership-issue-2)），也会让压缩记录的每次改动都牵动核心代码。F1 把这项检查移交给它的所有者：

| 部件 | F1 之后检查什么 |
| --- | --- |
| Timeline（`context_with_checkpoint`）与两个存储 | 只检查结构：锚点事件存在于快照的序号上、事件 id 与快照一致、解释器标签匹配、序号在切点之内（另有对解码后检查点的现有恢复位置检查）。不匹配 `EventKind` 或结局，SQL 中也不再出现业务类型或状态 |
| 快照所有者（`HistoryRenderer`，由 ContextPolicy 保存其快照） | 锚点是否有效，经 Timeline 本来就会调用的解释器回调判断。回调现在还接收锚点事件：`decode: fn(&RecoverySnapshot, &AgentEvent) -> Option<ProjectionCheckpoint>`。它只接受已完成的 `context/compaction`，且其 `retain_from` 与状态一致、`through` 早于快照。返回 `None` 就是缓存未命中，与现在相同 |
| 保存 | 所有者只在自己已完成的压缩处保存快照，与现在相同；存储不再强制检查类型 |

有效快照的行为不变；无效快照算作缓存未命中，退回完整的事实重放。

读取方随结构体更新：

- ContextPolicy 的重试过滤改为把 `turn_id` 与 `Some(turn)` 比较。
- 原生存储只按 `through` 索引已完成的压缩。
- 冻结的 v1 样本不改。它们只按 v1 事件解码；原生记录不接受旧的 `cursor`/`summary` 形式，v1 连接也不再收到 `context/compaction`（§12）。

**动态事件（决定 A）。** `context/compact` 和 `context/compaction-start` 是 `EventKind` 变体。#2 不迁移它们：#2 以同一信封、不透明数据承载核心未定义的事件类型，核心事件保留变体与 wire 形式。`context/compact` 由 Loop 拥有，`context/compaction-start` 由压缩事实的所有者拥有。

**诊断（破坏性修改）。** `CompactionDiagnosticsRecord` 改为与策略无关，原来面向摘要的必填字段移入一个由策略拥有的对象：

```text
{ reason: "auto" | "manual", trigger,           // 公共部分
  retainFrom: string, retainLimit?: string,    // Timeline 事件 id
  estimatedInputTokens, estimatedOutputTokens,  // 压缩前后，由 count_tokens 算出
  entriesReplaced, modelCallCount, durationMs,
  strategy: { kind: string, ... } }             // 由策略拥有
// 摘要策略: { kind: "summary", summaryChars, estimatedSummaryTokens,
//             chunkCount, compactedToolPayloadCount, droppedMessageCount }
```

公共字段由 ContextPolicy 填写，策略返回自己的对象。旧的扁平记录直接删除，不与新记录并存。冻结的 v1 样本不改，在 v1 模块中保留它们自己的扁平记录类型（§12）。

**客户端可见性。** SessionBroadcast 不加过滤地转发每个分支事件（[session_broadcast.rs](../../../../crates/dim-agent-capabilities/src/session_broadcast.rs)），但 Rpc 的 wire 过滤器——[tree_protocol.rs](../../../../crates/dim-app-server/src/tree_protocol.rs) 中的 `ProductEvent::new`——目前会丢掉 `context/compaction`。F1 经 `ProductEvent::new` 让协议 v3 连接能看到 `context/compact`、`context/compaction-start` 和 `context/compaction`，客户端由此可以从实时事件或历史中看到每一种状态。`context/budget` 和 `context/prepare` 仍然隐藏。

### 4.4 Rust 绑定

```rust
// dim-agent-types/src/events.rs
EventKind::ContextCompact(ContextCompactData {})                  // "context/compact"
EventKind::ContextCompactionStart(ContextCompactionStartData {     // "context/compaction-start"
    turn_id: Option<String>, request_id: Option<String>, command: Option<u64>,
    through: u64, trigger: CompactionTrigger, strategy: String })
pub struct ContextCompactionData {
    pub turn_id: Option<String>, pub request_id: Option<String>,
    pub command: Option<u64>, pub start: Option<u64>, pub through: u64,
    pub outcome: ContextCompactionResult }
pub enum ContextCompactionResult {
    Completed { retain_from: String, replacement: Vec<Message>, strategy: String,
                diagnostics: Box<CompactionDiagnosticsRecord>,
                context_baseline: Vec<ContextBaselineEntry> },
    Skipped { .. }, Failed { .. }, Cancelled { reason: String } }

// dim-agent-capabilities/src/host.rs
pub trait ContextModel { fn context_window(&self) -> f64; fn output_tokens(&self) -> f64;
                         fn threshold_tokens(&self) -> f64;
                         fn count_tokens(&self, r: &ModelRequest) -> f64; }   // 移除 compact
pub struct HistoryEntry { /* 现有 */ pub source: String }   // 放置它的事件的 id
pub trait CompactionStrategy: Send + Sync + 'static {
    fn name(&self) -> &str;                            // 写入开始事实，例如 "summary"
    fn compact(&self, input: CompactionInput, cancel: CancellationToken)
        -> HostFuture<'_, StrategyOutcome>; }          // Completed | Skipped
pub struct CompactionStrategyHost(pub Option<Arc<dyn CompactionStrategy>>);
pub struct CompactionCheckpoint { pub retain_from: String, pub replacement: Arc<[Message]>,
                                  pub context_baseline: Arc<[ContextBaselineEntry]> }

// dim-agent-capabilities/src/context_policy.rs（crate 内部）
impl ContextPolicy { async fn compact_step(&self, step: CompactionStep, cancel: CancellationToken)
                         -> Result<Arc<AgentEvent>, HostError>; }   // 已提交的事实

// dim-agent-capabilities/src/agent_loop.rs
impl AgentLoop { pub fn compact(&self) -> Result<u64>; }  // 追加 context/compact
impl PublicService for AgentLoop {}
// dim-agent-capabilities/src/context_preparation.rs
impl ContextPreparation { pub async fn usage(&self, cancel: CancellationToken)
                              -> Result<ContextUsage, ContextOpError>; }
impl PublicService for ContextPreparation {}

// dim-agent-capabilities：context_model.rs、summary_strategy.rs、context_methods.rs（新）
pub struct ProductContextModel;                    // ResolveContextModel，无依赖
pub struct ProductSummaryStrategyPlugin { host: DepKey<CompactionStrategyHost> }   // new(names)
//   Deps = (Keyed<ModelHost>, Keyed<ProviderConnectionResolver>)；提供 CompactionStrategyHost
pub struct ContextMethodsPlugin;
//   Deps = (Keyed<TreeCommands>, Keyed<SessionMethods>)；methods.register(ctx, "session.context", ..)

// dim-app-server/src/cli.rs 的 defaults::install_compaction，每种会压缩的配置共用这一处安装；
// 产品配置传入 ProductContextModel 和产品 home，--models 配置传入 AimuxModels 和它的 --home：
builder.session_value(Arc::new(ContextModelHost(Some(model))))
       .require_feature(Feature::new("ContextCompaction")
           .session("ProductSummaryStrategy", |names| names.part(ProductSummaryStrategyPlugin::new(names)?))
           .session("ContextMethods", |names| names.part(ContextMethodsPlugin)));
// mock 配置绑定 ContextModelHost(None) 和 CompactionStrategyHost(None)。
```

## 5. 流程

### 5.1 ProductContextModel

`resolve(request)`：

1. 要求 `request.capability_snapshot` 存在且对请求的模型有效。没有时返回 `InvalidRequest`“Model capabilities are not adopted; select the model again”，条件和文本与 `ProductResolver` 相同。
2. 返回绑定该请求的窗口事实：

| 方法 | 值 |
| --- | --- |
| `context_window` | `snapshot.capabilities.contextWindow`，即目录值，含其现有覆盖 |
| `output_tokens` | 有 `request.config.maxTokens` 时取它，否则取 `snapshot.capabilities.maxOutputTokens`；不超过后者 |
| `threshold_tokens` | `B − reserve`，其中 `B = context_window − output_tokens`（输入预算），`reserve = min(max(0.10 × B, 16,000), 0.20 × B)`。余量用来吸收下一个请求的估算误差；它是暂定的经验值，不能覆盖每一个没有提供方基线的请求（[研究 §3.3、§6.4](../research/compaction-trigger.md#33-建议)） |
| `count_tokens` | 序列化后的系统提示、消息和工具声明的 UTF-8 字节数除以 4；每个图片部件按固定 1,600 token 计。暂定（#128）。用于：在提供方的 `prompt_tokens` 之上算增量、第一个请求、`validate`、以及检查压缩是否减少了输入 |

`--models` 配置从模型文件解析同样的窗口事实：`AimuxModels` 按请求的 provider 与 model id 读取对应的模型条目；请求的能力快照保持未设置，所以能力未知时图片照常发出。

### 5.2 ProductSummaryStrategy

`compact(input, cancel)`：

1. **切点。** `retain_from` 是 `input.retain_limit` 或之前最晚的合法事件边界，通常就是 `retain_limit` 本身。这样的边界不存在时（单个很长的轮次），取最晚的、仍保留内容的合法边界；预算恢复（`provider_budget`）时也可以取替换全部的边界（#169）。仍然没有，或它之前什么都没有：返回 `Skipped {reason: "nothing_to_compact"}`，不调模型。
2. **摘要。** 对上一份替换内容（取自 `input.checkpoint`）加上由 `retain_from` 之前的事件放置的条目做摘要。
3. **结果。** 返回 `Completed {retain_from, replacement: [summary_message(summary)], strategy: "summary", diagnostics}`。现有的 `summary_message` 包装会把文本标为数据并转义；它只在这里施加一次，随记录保存。

摘要的产出方式：

- **请求。** 每块一个摘要请求。用 `input.request` 的 provider、model、快照和 config，`maxTokens = model.output_tokens()`，系统提示是本模块拥有的固定摘要提示，一条包含记录文本的 user 消息，不带工具。#170 为这个 config 提出一个推理设置，并在提示中加入目标长度（§13）。
- **凭据与失败。** 连接来自 `ProviderConnectionResolver`，只在本次调用内使用。调用经 `invoke_with_recovery` 运行，即 ModelProvider 的失败与凭据策略。S4 抽出了它；如果 F1 先合入，由 F1 做同样的抽取。
- **输出。** 丢弃 thinking 输出。工具停止或空文本算失败。
- **记录文本。** 按角色标注的文本：工具调用写名称和输入，工具结果写文本，图片写 `[image]`。
- **分块。** 一次调用的输入预算为 `context_window − output_tokens`，用 `count_tokens` 对整个摘要请求（系统提示、携带的摘要、外层格式和本块）计数，与其他所有请求的计数方式相同，不另设按字符计的规则。更长的记录分块折叠：每次调用拿到当前摘要和能放下的条目。单条放不下的，按字节在 UTF-8 字符边界上从中间截断，截断标记计入预算，并计入 `compactedToolPayloadCount`。
- **被截断的回复。** 停在 `max_tokens` 的回复（思考可能用掉大部分额度）不算摘要。该块在最接近中间的条目分隔处（或字符边界）一分为二，逐半折叠；小于 2 KiB 仍被截断时压缩失败，任何记录都不会保留被截断的文本（#169）。每一半都携带前一半的摘要，所以调用前会再次按预算检查它的整个请求；不再放得下的一半继续一分为二，文本全部保留；携带的摘要与包装在没有任何转录内容时也放不下，则压缩失败，不发送该请求。
- **诊断。** 摘要对象（`kind: "summary"`、`summaryChars`、`estimatedSummaryTokens`、`chunkCount`、`compactedToolPayloadCount`、`droppedMessageCount`、`reasoning`、`targetTokens`（没有指令时不出现）、`cutOffCount`）；公共部分由 ContextPolicy 填写。

### 5.3 一条压缩路径（ContextPolicy）

`compact_step` 就是把 `ContextPolicy::prepare` 中现有的压缩部分抽出来：

1. **保留界限。** 保留预算为 `0.25 × B`（暂定，#128）。`retain_limit` 是最早的、仍保留内容且保留尾部放得进该预算的合法切点的 `retain_from`；尾部用 `count_tokens` 对切点处的带 key 基线加上切点起的条目计数。基线要计入，因为每次切割后都会重放它。这样，最近的整轮放得下时整轮保留，正在进行的轮次过大时在它内部切割。手动步骤在最新一轮的开头切割，若按 token 规则的切点保留得更少则取后者。没有符合条件的切点时（单个条目或基线本身就超出预算），没有界限，按 `CompactionInput::cut` 的退回规则取切点。
2. **开始。** 追加 `context/compaction-start`，带调用方的 id、`command`、`through`、`trigger` 和 `strategy.name()`。
3. **策略。** 调用 `strategy.compact`，传入 `retain_limit`、合法切点 `cuts` 和 `trigger`。若在策略返回之前被取消，就为该开始追加 `cancelled` 结果并返回 `Cancelled`。
4. **验收。** 对任何策略都检查：
   - `retain_from` 是固定上下文中的一个事件，并通过基于事件的合法性规则（§4.3）；
   - `retain_from` 在 `retain_limit` 或之前；没有这样的合法切点时，它要保留内容，只有预算恢复时才能替换全部；
   - 投影后的请求比之前小，且低于窗口。

   任何一项不通过，结果变为 `failed`。
5. **结果。** 带上调用方的 id、`command` 和 `start` 追加 `context/compaction`，然后保存可选快照。

调用方：

- **ContextPreparation，在 `prepare` 内。** 达到阈值时（与现在一样，每个边界一次），或在 `recovery` 下。`prepare` 的其余部分不变。
- **Loop 的手动步骤。** 触发原因为 `Manual`。请求按 `start_preparation` 的方式构造：最新 `context/configured` 的设置和 system 加上投影后的消息，不绑定工具。

**预算恢复。** ModelProvider 在每次模型调用前调用 `validate`。它使用与触发点相同的请求估算（有提供方基线时为基线加增量，否则为完整的本地计数），超过 `B` 时带预算事实失败；随后 Loop 带 `recovery` 重试该请求一次。因此恢复的含义是“请求放不进声明的预算”，与触发点一致。没有提供方基线时，完整本地估算及其误差依旧存在（#128）。

### 5.4 手动压缩

**在 turn 内：**

```
      step/model-start {T, R1} ... step/model-output-end {T, R1, completed}
 k    context/compact {}                          写入方：Loop（AgentLoop::compact）
      -- Loop 消费 k：有在途请求 -> 待处理
      step/tool-call {T, R1} ... step/tool-result {T, R1}
      -- Loop 消费工具结果（边界 b）：待处理 -> 压缩步骤
 s    context/compaction-start {turn_id: T,       写入方：ContextPolicy（Loop 的步骤）
        command: k, through: b, trigger: manual, strategy: "summary"}
 m    context/compaction {turn_id: T, command: k,  写入方：ContextPolicy（Loop 的步骤）
                          start: s, through: b, outcome}
      -- Loop 消费 m：步骤结束 -> 照常 next_model
      input/consumed* ; step/model-start {T, R2}   Loop（不变）
      context/prepare {T, R2, ...} -> model/call   基于压缩后的上下文
```

**没有 turn 在运行：**

```
 k    context/compact {}                          写入方：Loop（AgentLoop::compact）
      -- Loop 消费 k：没有在途请求 -> 立即运行压缩步骤
 s    context/compaction-start {command: k, through: k,      写入方：ContextPolicy
                                trigger: manual, strategy: "summary"}
 m    context/compaction {command: k, start: s, through: k, outcome}   写入方：ContextPolicy
      -- Loop 消费 m：步骤结束 -> try_start
      （没有 turn/start，没有 turn/end；步骤期间应用的驱动输入此时启动 turn，
       基于压缩后的上下文准备）
```

**对照：自动压缩（在请求的准备中）：**

```
      step/model-start {T, R} ; context/prepare {T, R, through: p}       Loop
      context/budget {T, R}                                              ContextPreparation
 s    context/compaction-start {turn_id: T, request_id: R, through: p,   ContextPolicy
        trigger: threshold | recovery, strategy: "summary"}
 m    context/compaction {turn_id: T, request_id: R, start: s, through: p, outcome}
      model/call {T, R}                                                  ContextPreparation
```

**调度规则：**

- **边界。** 步骤边界是 Loop 本要准备下一个请求的地方（不带预算重试 source 的 `next_model`），或 turn 结束的地方。待处理的步骤在第一个边界启动。
  - turn 结束而不是继续时，步骤在 `turn/end` 之后运行，不属于任何 turn。
  - 预算重试属于它的请求，不算边界。
- **through。** `through` 是触发步骤启动的那个被消费事件的序号。
- **至多一条未完成指令。** 按维护者的决定，重复请求属于交互层面的问题：去重在交互边界完成，不由 Loop 的状态机或 Timeline 处理。
  - `AgentLoop::compact` 是 `context/compact` 的唯一写入方。它在 Loop 的锁内检查该分支上是否有未完成的指令（已写入、尚无结果）。有则不写入，返回该指令的序号，调用方等待该指令的结果；否则写入指令。
  - Loop 的压缩槽至多保存一条未完成指令，由它的结果清除。
  - **防御规则。** Loop 在另一条指令未完成时消费到的 `context/compact`（不是经 `AgentLoop::compact` 写入的，或旧数据），立即以引用它的 `context/compaction {command: <其序号>, outcome: skipped {reason: "a compaction is already in progress"}}` 结算，不带开始，不排队。
- **不推进。** 步骤运行期间，`next_model` 和 `try_start` 等待它的事实。输入照常排队，不被步骤消费。

**steer。** 步骤期间应用的驱动输入不取消步骤。它加入下一个请求，或在步骤结束后启动一个 turn。中断、失败、重载与重启见 §5.8。

### 5.5 Loop 新增的内容

全部在 `crates/dim-agent-capabilities/src/agent_loop.rs`：

1. **订阅。** 加入 `context/compact`、`context/compaction-start` 和 `context/compaction`；`Deps` 加入 `CompactionStrategyHost`。
2. **公开写操作。** `AgentLoop::compact()` 返回未完成指令的序号，或追加 `context/compact` 并返回其序号（§5.4）。`AgentLoop` 加上 `PublicService` 标记。
3. **压缩槽**，至多一条未完成指令。消费这条指令时，Loop 把步骤标为等待；没有在途请求时立即启动。另一条指令未完成时消费到的指令被跳过（§5.4）。
4. **边界。**
   - `next_model`（带预算重试 source 时除外）和 `turn/end` 有待处理步骤时先启动步骤，而不是推进。
   - 步骤运行期间，`next_model` 和 `try_start` 什么都不做。
5. **步骤任务。** 与准备任务一样受跟踪；按 `start_preparation` 的方式构造请求，调用 `ContextPolicy::compact_step`。
6. **步骤终态。** 消费到未完成指令的结果时，Loop 清空压缩槽；若其步骤运行过，Loop 继续：在 turn 内调用 `next_model`，否则调用 `try_start`。其他指令的结果（跳过、结算）不改变任何状态。
7. **中断。** `stop/interrupt` 取消正在运行的步骤，其结果为带 `start` 的 `cancelled`；或为等待中的指令写不带 `start` 的 `cancelled`。该指令只结算一次。
8. **重建。** 压缩槽由指令、开始与结果的配对折叠：
   - 有手动开始而没有结果：步骤当时在运行。Loop 写带该 `start` 和 `command` 的 `cancelled` 结果。
   - 有指令而既没有开始也没有结果：指令当时在等待。Loop 写带该 `command`、不带 `start` 的 `cancelled` 结果。每条指令只结算一次。正在卸载的 Loop 不再启动步骤。

不变的部分：turn 与请求的生命周期、输入消费、工具批次、对模型请求的 steer、预算恢复、`LoopSnapshot` 和 `context/prepare`。

### 5.6 ContextPreparation 新增的内容

- `ContextPolicy::prepare` 改为调用 `compact_step`。ContextPreparation 注入 `CompactionStrategyHost`；两个宿主都是 `Some` 时它才有策略。
- `rebuild`：没有结果的自动开始（没有 `command`）先以 `cancelled` 结果结算，然后照旧以重载终态结算未完成的准备。
- `usage(cancel)` 读取已提交头部，运行现有估算。不写任何东西。

### 5.7 客户端看到的压缩状态

每种状态都是 Timeline 事实，客户端任何时候都可以从实时事件或历史中看到：

| 状态 | 事实 | 适用于 |
| --- | --- | --- |
| 待处理 | 有一条 `context/compact`，但还没有引用它的开始或结果 | 手动 |
| 运行中 | 有一条 `context/compaction-start`，但还没有它的结果 | 手动与自动 |
| 已结束 | `context/compaction`（`completed`、`skipped`、`failed`、`cancelled`） | 手动与自动 |

- 在压缩中途接入的客户端读取分支历史（`session.history` 或接入时的重放），看到一条没有结果的开始，就能显示“运行中”，不需要额外的通道。
- 开始带 `trigger`（manual、threshold 或 recovery）和 `strategy`，结果带结局。客户端不需要其他状态。
- wire 过滤器的改动见 §4.3。

### 5.8 失败、中断、重载与重启

| 情况 | Timeline 事实 | Loop 接下来做什么 | 客户端看到什么 |
| --- | --- | --- | --- |
| 策略错误或验收失败，turn 内的手动压缩 | 开始，然后是 `context/compaction {start, failed}` | 不压缩，继续该 turn（`next_model`）。下一个请求超过输入预算时，由现有的预算恢复强制压缩（退回切点）；这次也失败的话，请求和 turn 失败 | 运行中，然后是 `failed`；回复 `compaction_failed` |
| 同上，turn 外的手动压缩 | 开始，然后是 `failed` | 不再做别的（有输入在等时调用 `try_start`） | 运行中，然后是 `failed`；回复 `compaction_failed` |
| 同上，自动压缩 | 开始，然后是 `failed` | 不变：请求不压缩继续；在恢复中则请求失败 | 运行中，然后是 `failed` |
| 手动，无法构造请求（没有配置或快照） | 不带 `start` 的 `failed` | 同上 | 待处理，然后是 `failed` |
| 正在运行的步骤（手动或自动）被 `stop/interrupt` | 该开始的 `cancelled {reason}`，策略已经返回时为 `completed` | turn 内的手动压缩：经 `next_model` 检查，turn 以 `Cancelled` 结束。自动压缩：请求照旧以 `Cancelled` 结束 | 运行中，然后是 `cancelled` 或 `completed`；回复与之一致 |
| 手动指令待处理时被 `stop/interrupt` | 不带 `start` 的 `cancelled` | turn 的行为不变 | 待处理，然后是 `cancelled` |
| 插件重载 | 对每条没有结果的开始，由其所有者写带该 `start` 的 `cancelled: "capability reloading"`：手动开始由 Loop 写，自动开始由 ContextPreparation 写（在现有重载终态之前）。待处理的指令由 Loop 写不带 `start` 的 `cancelled` | 不重放。被重建的 turn 照旧以 `Failed` 结束 | 运行中或待处理，然后是 `cancelled`；等待中的回复为 `operation_interrupted` 或 `compaction_cancelled` |
| 压缩中途进程崩溃或重启 | 下次装载时，从同样的未配对开始和指令出发，按重载的方式结算 | 同重载 | 装载前从历史看到运行中，之后是 `cancelled` |

**关于崩溃与重启：**

- **事实是原子的。** Timeline 批次原子提交，所以开始或结果要么完整存在，要么不存在，不会只写一半。在两者之间崩溃会留下一条没有结果的开始，由下次装载结算。
- **快照只是缓存。** 在事实与投影快照之间崩溃不会造成问题：投影从事实重放（原则 §4 规则 9）。
- **模型工作会丢失。** 进行中的策略模型工作会丢失。下一次自动触发或新的 `/compact` 会重新压缩。
- **不续做半途的压缩。** 运行时在任何地方都不会在重载后重放模型调用，因为模型调用不是幂等的。而且用户的意图可能已经过时：历史可能已经增长，用户也可能不再需要它。

## 6. 错误

`MethodError` 代码（未注明的为 `code -32000`）：

| 代码 | 方法 | 何时 | 重试 |
| --- | --- | --- | --- |
| `-32602` | 两者 | 缺少 `branchId` 或不是字符串 | 否 |
| `session_invalid_input` | 两者 | 未知分支 | 否 |
| `session_closed` | 两者 | Session 或分支正在关闭 | 否 |
| `context_not_configured` | usage | 分支历史中没有 `context/configured` | 采用模型之后 |
| `context_model_failed` | usage | `resolve` 失败（没有能力快照） | 重新选择模型之后 |
| `compaction_failed` | compact | 事实为 `failed`：共享失败策略之后的策略错误、空摘要、未通过验收的 `retain_from`、没有减少输入、没有 `context/configured`，或没有能力快照 | 由用户决定 |
| `compaction_cancelled` | compact | 事实为 `cancelled`：`stop/interrupt` 或重载 | 由用户决定 |
| `operation_interrupted` | compact | 等待期间 ContextMethods 卸载；指令照样会被服务或结算 | 由用户决定 |

自动压缩的错误处理不变：策略错误成为 `failed` 事实，请求不压缩继续；在恢复中则使请求失败。

## 7. 安全

- **凭据只在请求内。** 连接在一次策略调用内从 `ProviderConnectionResolver` 取得并交给 `ModelHost`。它从不被保存、返回、序列化或记录。Provider 不变。
- **没有新的数据外发。** 摘要调用把 Session 自己的历史发给每个 turn 都会收到它的提供方和模型。
- **替换内容是不可信的模型输出。** 摘要策略把它放在现有的 `summary_message` 包装里保存，该包装把它标为数据。
- **结果中没有密钥。** 事实、回复和诊断只含计数、id、替换消息和 `HostError` 文本，后者不含凭据值。请求和摘要内容不进 tracing。
- **授权。** Rpc 对 Session 方法校验 `sessionId`（C1）。

## 8. 与现有部件的关系

| 部件 | 关系 |
| --- | --- |
| Loop | 补充（§5.5）。产品配置下预算恢复生效 |
| ContextPreparation、ContextPolicy | 补充（§5.3、§5.6）：`compact_step`、保留规则、对任何策略的验收、`usage` |
| 投影（`history.rs`）、快照 | 用 `replacement` 和 `retain_from` 的一条重放规则；`HistoryEntry.source`；切点验收与快照状态改为基于事件（解释器版本升级）；解码回调检查锚点事件（#130） |
| Timeline 核心、内存与 SQLite 存储 | 快照锚定只做结构检查（#130，§4.3） |
| `HistoryRenderer` | 每次请求都执行、不留记录的渲染期投影不是压缩，例如按固定规则截断旧的工具输出。它们属于现有可注入的 `HistoryRenderer`，不属于策略 |
| ModelProvider | 代码不变。产品配置下 `validate` 开始按估算检查窗口 |
| Provider、ProductResolver、Aimux | 不变；摘要策略经 `ProviderConnectionResolver` 和 `ModelHost` 使用它们 |
| AuxiliaryModel（S4） | 不使用。共用 `invoke_with_recovery`；谁先合入谁负责抽取 |
| `sessionMethods`（C1） | 两个方法需要它。宿主值和策略不需要 C1 |
| Rpc `ProductEvent`（`tree_protocol.rs`） | 让 v3 连接能看到 `context/compact`、`context/compaction-start` 和 `context/compaction` |
| turn 的消费方（客户端、将来的 Goal、Hooks） | 不受影响。turn 之外的手动压缩不写 `turn/*` |
| #2 动态事件 | 不变：两个事件保留为 `EventKind` 变体，wire 形式不变 |
| `--models` 配置 | 同样的策略和 `session.context` 方法；窗口事实来自模型条目，请求的能力快照保持未设置 |
| mock 配置 | 两个宿主都是 `None`：不压缩，也不登记 `session.context` |
| ManagedProviders 套餐用量、CLI `/compact` 与 `/usage` | 不在 F1 的验收内 |

## 9. 规则检查

| 规则 | 回答 |
| --- | --- |
| 原则 §1（Capability、App Server 边界） | 新部件和补充都在 `dim-agent-capabilities`；App Server 只在产品配置中安装它们。 |
| §1 唯一事件总线 | 指令和事实都在分支 Timeline 上，经其 Dispatcher 消费。方法的等待读取的也是这个 Timeline。客户端从同样的事实看到压缩状态，没有旁路。 |
| §3 依赖方向 | 新部件依赖核心服务。ContextPreparation 和 Loop 依赖 `ContextModelHost` 与 `CompactionStrategyHost` 两个键，每种配置都显式绑定它们。没有可选依赖。 |
| §3 改动其他模块（Loop） | 已批准的改动。需求来自 Loop 自己对上下文窗口的控制：压缩是 Loop 的一个步骤，和模型调用、工具批次一样，Loop 不能在一个将被替换内容取代的上下文上推进。它在 Loop 内闭环：自动压缩本来就在 Loop 的请求准备中运行，手动步骤在同一个排序者之下使用同一个函数。 |
| §3 不改动即可扩展（三层） | 机制（Loop 步骤与触发）和合同（记录与重放）固定，策略是可替换的 Session 值。新增策略既不改 Loop，也不改记录和重放；重放旧记录从不调用策略（§2.2）。 |
| §4 规则 1（指令事件） | 客户端用 `context/compact` 请求推进过程的所有者。Loop 决定何时服务它，结果是一条引用该指令的 `context/compaction`。 |
| §4 规则 2（交接） | 模型请求仍以 `context/prepare` 交接。手动步骤不交接任何东西。 |
| §4 规则 3（before 回调；处理器） | 不用 before 回调。Loop 的处理器只登记步骤任务就返回。 |
| §4 规则 4（核心无业务含义） | Timeline、Dispatcher 和门面不变。 |
| §4 规则 5（原子批次） | 每次新写入都是一个事件。 |
| §4 规则 6（上下文只来自交接后的事件） | 替换内容只经已完成事实的投影到达模型。待处理输入不被步骤消费（AGENTS 规则 10）。 |
| §4 规则 7（公开事件与服务） | `TreeCommands`、`PublicService` 上的 `Branch::service`、`ProviderConnectionResolver`、`ModelHost`、`sessionMethods`。 |
| §4 规则 8（基础设施不变） | Dispatcher、信封、rutis、loader、Agent 装配和门面都不改。有一处基础设施改动（#130）：Timeline 与两个存储的快照锚定不再了解 `context/compaction`，只保留结构检查，所有者的解码回调接收锚点事件。需求来自 Timeline 自身：它不能了解事件内容（原则 §4 规则 4；Timeline 只检查信封与核心保留领域，ADR 001）。它在 Timeline 自己的合同内闭环：快照是带结构锚点的缓存。有效快照的行为不变，无效快照算作缓存未命中。|
| §4 规则 9（只写需要的事实） | `context/compact` 由 Loop 后续的决定和重载读取。`context/compaction` 用于投影。按维护者的决定，`context/compaction-start` 和可见的结果都是事实，理由有两条。第一，它们不只是驱动某个状态机：用户和其他状态机读取历史时需要这些内容（任何时候接入的 UI 都必须看到压缩正在进行，或者它是怎样结束的）。第二，重载结算要用它们：没有结果的开始意味着当时有压缩在途。Loop 和 ContextPreparation 由开始与结果的配对重建，这比只靠指令更精确：它能区分运行中和待处理，也覆盖自动压缩。用量查询不写任何东西。 |
| ADR 002 D1（作用域） | 窗口事实、策略和方法：Session。指令、步骤和压缩槽：分支，在 Loop 中。 |
| ADR 002 D2（无接线模块） | ContextMethods 经 `sessionMethods` 暴露所有者的操作。 |
| ADR 002 D3（依赖） | 只硬依赖核心。Provider 或 Aimux 重载会传播到策略部件（已接受的代价）。 |
| ADR 002 D4（配置） | 不增加配置；暂定常量写在代码里（#128）。 |
| ADR 002 D5（语言中立合同） | §4.1 与 §4.3 是合同；§4.4 是一种绑定。 |

**影响面（公开内容）：**

- `session.context` 命名空间。
- 指令 `context/compact`。
- 改动后的 `context/compaction`（可选 id、`command`、`start`、`cancelled`，以 `retain_from` 代替 `cursor`，以 `replacement` 和 `strategy` 代替 `summary`，与策略无关的诊断）。
- 事实 `context/compaction-start` 及结果上的 `start` 引用。
- `context/compact`、`context/compaction-start` 和 `context/compaction` 对 v3 客户端可见。

内部 Rust 改动：

- 拆分后的端口与 `CompactionStrategyHost`。
- `CompactionInput` 的字段。
- `CompactionCheckpoint {retain_from, replacement}` 与 `HistoryEntry.source`。
- `history.rs` 的切点验收与游标查找改为基于事件；快照状态保存 `retain_from`（解释器版本升级）。
- 快照锚定（#130）：`Timeline::context_with_checkpoint` 只保留结构检查，其 `decode` 回调改为 `fn(&RecoverySnapshot, &AgentEvent)`；`MemoryTimelineStore::save_snapshot` 以及 SQLite 存储的保存与查找都去掉压缩类型与状态的检查。
- `AgentLoop` 与 `ContextPreparation` 上的 `PublicService`。

**向机制所有者提出的需求：** 无。

## 10. 决定与待定问题

| 项 | 决定 |
| --- | --- |
| 用哪个模型压缩 | 摘要策略用请求自己的模型；单独的模型推迟 |
| 触发点与保留量 | #128：在 `B − min(max(0.10 × B, 16,000), 0.20 × B)` 触发；保留放得进 `0.25 × B` 的最长尾部（含带 key 的基线）；手动步骤至多保留最新一轮；`validate` 用触发点的估算与 `B` 比较；`字节数 / 4` 和每张图片 1,600 token 不变。余量和份额是暂定的经验值 |
| 手动压缩 | 由 `context/compact` 安排的 Loop 步骤，在 turn 内外都可以，永远不创建 turn |
| 步骤在哪里运行 | 在 Loop 中，调用 ContextPolicy 唯一的压缩函数（§2.1） |
| 记录 | `retain_from`（Timeline 事件 id）、`replacement` 与 `strategy`，一条重放规则；可选 id、`command`、`cancelled`；直接修改（尚未发布） |
| 端口 | `ContextModel`（窗口事实）与 `CompactionStrategy`（替换内容），各来自自己的 Session 值 |
| 其他策略、组合 | F1 不做 |
| 新事件类型 | `context/compact` 与 `context/compaction-start` 作为 `EventKind` 变体 |
| `--models` 配置 | 窗口事实来自模型条目；能力快照保持未设置 |
| 提供方报告的溢出 | 单独的 Aimux issue，不在 F1 内 |
| 诊断 | 现在就改成与策略无关：公共计数加一个由策略拥有的对象（破坏性修改） |
| 压缩状态 | 事实：待处理 = 有指令无开始，运行中 = 有开始无结果，已结束 = 结果（§5.7） |
| 半途的压缩 | 从不续做；结算为 `cancelled`（§5.8） |
| 摘要器推理（#170） | 由评估决定（§13.4）：`compaction.json` 中的 `summaryReasoning: inherit \| off \| low`，或在一个值于每类模型上都胜出时改为常量。在此之前为 `inherit`（现在的行为） |
| 摘要目标长度（#170） | 由评估决定（§13.4）：在提示中写明 `S = min(α × O, β × B)`，或不写指令。在此之前不写指令 |

待定：F1 之外的 #128 后续工作（复用对话前缀的摘要请求、Anthropic 缓存断点、把提供方报告的溢出映射到恢复），以及运行 #170 的摘要评估（§13），其结果确定上面两行。

## 11. 验收（流程测试）

测试用产品配置的 App Server 连接一个上下文窗口很小的 mock HTTP 提供方，也用同样的 App Server 连接一个模型条目声明了小窗口的 `--models` 文件（`crates/dim-app-server/tests`），另加 `crates/dim-agent-capabilities/tests` 中的 Loop 与策略流程测试。

1. **自动。** 超过阈值后，下一次准备依次写 `context/budget`、`context/compaction-start {trigger: threshold}`，并发出一个摘要请求。它提交 `completed` 的 `context/compaction`：带该请求的 id 和 `start`，没有 `command`，`strategy: "summary"`，`replacement` 只有一条消息，`retain_from` 为保留尾部放得进 `0.25 × B` 的最早合法切点。模型请求以这条消息开头，原样保留该尾部。产品窗口事实给出余量的上限、下限和比例（32k/8k → 19,200；128k/8k → 104,000；1M/128k → 784,800）。一个大轮次之后的许多小轮次会保留两轮以上；切点后会重放的带 key 块，比同样内容不带 key 时保留的轮次少。
2. **预算恢复。** 重试时以 `trigger: provider_output_budget_exhausted` 压缩。有提供方基线时，本地计数放得进窗口、但基线估算超过 `B` 的请求会恢复；基线估算放得下的不会。
3. **手动，没有 turn。** `session.context.compact` 只写 `context/compact`、`context/compaction-start {command}` 和 `context/compaction {command, start}`；没有 `turn/*`、`step/*` 或 `model/call`。回复为 `completed`。两轮的 Session 上切点为最新一轮的开头；该轮的输入本身超出保留预算时，切点在这一轮内部。
4. **手动，turn 内。** 序列为 `step/tool-result` → `context/compaction-start {turn_id, command}` → `context/compaction {turn_id, command, start}` → 下一个请求的 `step/model-start`，该请求基于压缩后的上下文准备。
5. **重放与替换策略。**
   - 压缩之后重载的 Session、以及压缩之后 fork 出的分支，投影出的消息与之前相同。
   - 把摘要策略换成一个返回两条 `replacement` 消息的测试策略：其结果按同一条规则提交和重放，Loop 和策略层代码都不用改。
   - `retain_from` 落在保留尾部之内、或把工具调用与其结果分开的测试策略，会被策略层拒绝。
   - 用默认 `HistoryRenderer` 压缩的 Session，换一个渲染器（另一种条目 id 方案）重放时，仍能凭 `retain_from` 找到切点，投影出相同的保留事件。
   - 快照锚定（#130）：两个存储对锚定在任何现有事件上、id 与序号匹配的快照都照常保存和返回。所有者的解码回调会忽略锚点不是已完成压缩、或 `retain_from` 与其状态不一致的快照，上下文随之退回事实重放，结果相同。
6. **中断、重载与重启。**
   - 步骤期间 `session.stop` 写 `cancelled`（策略已经返回时为 `completed`）。
   - 步骤期间插件重载写 `cancelled: capability reloading`，不发第二次策略调用。
   - 在摘要请求被挂住时杀掉 App Server 进程，再在同一个 home 上启动：留下一条没有结果的开始，没有半截结果；下次装载写带该 `start` 的 `cancelled: capability reloading`，自动压缩同理；Session 投影出的历史与压缩前相同；新的 `/compact` 正常压缩。
   - 在事实之后、快照保存之前崩溃：从事实投影出压缩后的历史。
   - turn 内的手动压缩失败后 turn 继续；下一个请求超过输入预算时由预算恢复压缩。
7. **至多一条未完成指令。**
   - 有指令未完成时重复或并发调用 `session.context.compact`：Timeline 中恰有一条 `context/compact`、一条开始和一条结果，每个调用方收到相同的回复。
   - 另一条指令未完成时消费到的 `context/compact`（不经 `AgentLoop::compact` 写入）立即得到不带开始的 `skipped: "a compaction is already in progress"`，未完成的压缩照常完成。
   - 有一条未完成指令时的 `stop/interrupt` 或插件重载，把它结算为 `cancelled`，只结算一次。
8. **客户端看到的压缩状态。**
   - 接入该 Session 的 v3 客户端实时收到 `context/compact`、`context/compaction-start`（`trigger: manual`）和 `context/compaction`。
   - 在摘要请求被挂住时才接入的客户端，从分支历史（一条没有结果的开始）看到“运行中”。
   - 自动压缩显示 `trigger: threshold` 和请求 id。
   - 在以上所有测试中（成功、跳过、失败、中断、重载、崩溃），每条 `context/compaction-start` 都恰好有一条带其 `start` 的 `context/compaction`，也没有结果引用不存在的 `start`。

## 12. 实现说明

实现遵循本设计。代码需要而正文未定的细节，取最小的选择，记录如下。

| 项 | 实现 |
| --- | --- |
| 开始事实的触发值 | `trigger` 使用现有的 `CompactionTrigger`：`manual`、`threshold`、`provider_output_budget_exhausted`（§4.3 原写 `recovery`；§4.2 与验收 2 已用后者）。 |
| 合法切点 | `CompactionInput` 另带 `cuts`：按顺序列出每个合法切点（`retain_from`、第一个保留条目、是否在 `retain_limit` 之内），由 ContextPolicy 按基于事件的有效性规则算出。候选为轮次开始、放置条目的事件，以及边界的最后一个事件（替换全部）。`CompactionInput::cut()` 返回限制内最晚的切点；限制内没有时，返回最晚的、仍保留内容的切点（预算恢复时可以是任意切点）。策略可以返回任意 `retain_from`，策略层仍会校验。 |
| 采纳 | `compact_step` 在追加事实前校验策略结果，再采纳 Timeline 中间件处理后的已提交事实：投影、缩减检查和快照都使用已提交的 `retain_from`、`replacement` 与基线。已提交的切点无法投影时，本次准备失败，与此前已提交游标的行为一致。 |
| 诊断 | 策略找不到切点（`skipped`）时省略 `retainFrom`；没有替换任何内容时省略 `estimatedOutputTokens`。 |
| v1 | 生产代码中没有 v1 到原生的转换：冻结样本只按 v1 事件解码；`v1::EventRef` 对 `context/compact`、`context/compaction-start` 和变更后的 `context/compaction` 没有形式，因此 v1 连接不再收到压缩事实。旧的扁平诊断类型移入 v1 模块。 |
| Loop 订阅 | Loop 订阅 `context/compact` 与 `context/compaction`；开始事实只在重建时折叠，用来区分运行中的步骤和待处理的命令。 |
| 至多一条未完成指令 | 评审中维护者的决定：重复请求在交互边界去重。`AgentLoop::compact` 加入未完成的指令而不再写入新指令，压缩槽至多一条指令，另一条指令未完成时消费到的指令被跳过，等待方按自己的指令 id 匹配（§4.1、§5.4、验收 7）。这取代了此前的当前/待处理拆分，指令因此不会再与运行中的步骤交错。 |
| 快照锚点 | Timeline 保留 §4.3 的结构检查。原先的 `resume ≤ through` 需要读取锚点内容，改为结构性的“resume 在锚点所在分支上早于锚点”，`through` 早于锚点由所有者的解码回调检查。 |
| 原生存储示例 | [native-storage-format](../../../design/appserver/native-storage-format.md) 中的记录示例改用新记录和 v3 解释器。 |
| 摘要预算 | 评审修正：分块与截断用窗口事实的 `count_tokens` 对整个摘要请求计数，对照 `context_window − output_tokens`（§5.2），以 CJK 为主的历史不再超出窗口；`product_summary.rs` 检查每个摘要请求。 |
| 验收 | 验收 1、3、6（崩溃与重启）和 8 在二进制流程测试 `transport_flow/compaction.rs` 中运行。验收 2、4、5 以及 6 中的重载、中断和失败情形在 `dim-agent-capabilities/tests/context_flow`（`manual_compaction.rs` 和既有压缩测试）中运行。事实与快照之间的崩溃由 `compaction_events.rs` 的快照失败测试覆盖。 |
| 触发点、保留与恢复（#128） | `ContextPolicy` 保存其 `prepare` 返回的请求的估算（完成压缩后为完整计数），所以 `validate` 检查同一请求时不再读取上下文；不是它准备的请求会读取一次上下文。每个保留条目用 `count_tokens` 单独计数后求和，因此不含整体计数时消息之间的分隔符。触发点取值在 `transport_flow/compaction.rs` 和 `product_summary.rs` 中验证；保留与恢复在 `context_flow/retention.rs`；手动切点在 `manual_compaction.rs`。 |
| 死区与被截断的摘要 | #169：保留规则的退回不再依赖预算恢复，单个很长的轮次会在最晚的、仍保留内容的切点压缩；只有恢复时才能替换全部。`CompactionInput` 去掉了 `fallback`。停在 `max_tokens` 的摘要回复会对半重试，到最小块仍截断就失败，不再被保留。 |

## 13. 摘要质量：推理与目标长度（#170）

状态：[#170](https://github.com/arcships/dim/issues/170) 的已批准设计。本节设计评估方法和两个可调项；默认值由运行评估（§13.4）决定，评估跑完后记入 §10。在此之前，§5.2 的默认行为不变：摘要器沿用对话的推理设置，提示词不写长度。

### 13.1 这里决定什么、不决定什么

摘要策略有两个未定的选择。两者都是策略层（§2.2）的提示词与配置内容，不是机制，所以都不改 Loop、ContextPolicy、记录或重放（[研究 §5.4](../research/compaction-trigger.md)）。

| 选择 | 现状 [代码] | 为什么未定 |
| --- | --- | --- |
| 摘要请求的推理 | `summary_request` 复制 `input.request.config`，只设置 `maxTokens = output_tokens`（[summary_strategy.rs](../../../../crates/dim-agent-capabilities/src/summary_strategy.rs)），所以开着思考的对话得到的摘要器也会思考 | 思考与摘要正文共用 `max_tokens`，而且不是每个模型都能控制思考长度；#169 之后，停在 `max_tokens` 的回复会被对半拆分重试，所以思考带来的是调用次数和延迟，而不是正确性问题。思考能否提升摘要质量，没有测过 |
| 目标长度 | 固定的 `SYSTEM` 提示只说“简洁”，不写长度 | 写明长度可能约束之后每个请求都要携带的替换内容，可能减少 `max_tokens` 停止；也可能丢事实。这是提示词质量问题 |

不在范围内：单独的摘要模型（§10）、摘要请求复用前缀与缓存断点（#128 的后续）、以及对分块、截断规则、保留量或触发点的任何改动。

### 13.2 `summaryReasoning`

**合同。**

| 值 | 摘要请求的 config |
| --- | --- |
| `inherit` | 对话请求的 config，与现在相同 |
| `off` | 对话请求的 config，推理强度设为 `none`，按 Provider 为对话做的同一映射转换 |
| `low` | 对话请求的 config，推理强度设为 `low`，同样映射 |

- 该值作用于一次压缩的所有摘要请求，在压缩开始时读取一次。所有情况下 `maxTokens = output_tokens` 都不变。
- `off` 和 `low` 在模型允许时生效。Provider 的映射对该模型和协议拒绝这个强度时（不支持推理、该协议没有关闭推理的声明、`low` 不在模型的强度选项中、Bedrock、本地开发连接），请求使用 `inherit`。这就是该设置的业务含义：它是对摘要器的偏好，不是会让压缩失败的要求。没有推理能力的模型在 `inherit` 下本来就等同于 `off`。
- `low` 不看对话自己的推理设置：即使对话关闭了推理，它也请求强度 `low`。要跨协议比较一个请求的实际推理状态，需要第二套对提供方选项的解读，而只有 Provider 有这套解读。
- 实际采用的值写入摘要诊断对象的 `reasoning`（§13.5），记录里能看到摘要器实际怎样运行。没有发出摘要请求的压缩记录的是请求的值。

**放在哪里。** 摘要策略的业务配置（ADR 002 D4）：产品 home 中的用户文件 `compaction.json`，`{"summaryReasoning": "inherit" | "off" | "low"}`，拒绝未知成员。

- `ProductSummaryStrategyPlugin` 的工厂捕获产品 home 路径，`SummaryStrategy::new` 在加载时用 `FileStorage::register_root` 把它解析成 FileStorage 根：该调用返回 `serve` 已为该 home 注册的根（按路径幂等，不会新开根）；产品配置在 `cli.rs` 中组装，早于 `serve` 计算根列表，所以工厂无法携带那个根的索引。策略随后在每次压缩开始时经 `FileStorage` 读取该根下的 `compaction.json`。修改在下一次压缩生效，无需重载。文件不存在表示默认值（§13.4）。
- 文件不可读或无效时压缩失败（`failed`，回复 `compaction_failed`），消息点名 `compaction.json`，与无效的 `exec-shell.json` 让需要它的命令失败相同。这个文件是用户的明确选择，悄悄换成别的值会把它藏起来。
- 只有用户作用域。不提供项目文件：摘要器推理是用户的成本与质量偏好，不是某个仓库的。
- 它不是声明式 Settings 源（那是给非 Rust 所有者的），也不是带管理方法的 `BusinessDocument`：#170 中没有客户端编辑它。等客户端需要时，它以同样的文档变成带所有者方法的 `BusinessDocument`。
- mock 配置不安装该策略，所以没有这个文件。`--models` 配置用它的 `--home` 安装该策略，也就是其存储本来使用的 home，所以它的 `compaction.json` 放在那里。

**如何经过 Provider 映射。** 强度映射已经存在：`ModelPreferences::configure`（[preferences.rs](../../../../crates/dim-app-server/src/provider/preferences.rs)）按协议把 `reasoningEffort` 转成 config 字段。对 `none`，它使用模型的 `reasoning.off` 声明（`thinking_type: disabled`、`thinking_budget: 0`、`enable_thinking: false`），或模型声明的原生 `none` 强度；否则拒绝。目前只能经 App Server 中的 `ProviderService::configuration` 和 `apply_preferences` 用到它。

策略在 `dim-agent-capabilities` 中，不能调用 `ProviderService`；而只为策略适配 Provider 的部件会是接线模块（ADR 002 D2）。所以：

1. `ModelPreferences::configure` 的推理强度分支原样（行为不变）移到 `dim-agent-capabilities` 中 `ModelProtocol` 旁边的纯函数：
   `reasoning_config(base: &ModelConfig, snapshot: &ModelCapabilitySnapshot, protocol: ModelProtocol, effort: &str) -> Result<ModelConfig, &'static str>`。
   它读取 `snapshot.capabilities.reasoning` 和 `snapshot.reasoning`，正是 `configure` 现在读取的目录字段（`ProviderService::configuration` 把 `model.capabilities` 和 `model.metadata.reasoning` 复制进快照）。`configure` 调用它。Provider 的行为和错误不变；它现有的流程测试覆盖这次移动。
2. 策略照常为本块解析连接，从解析出的 HTTP 连接取 `protocol`，对复制的 config 应用 `reasoning_config`，再用该连接调用。连接解析读取请求的 provider、model 和快照，不读 config [代码：`ProductResolver`]，所以为沿用 config 解析出的连接就是映射后 config 的连接。`Err` 表示 `inherit`（见上）。

### 13.3 目标长度

**指令。** 有目标时，系统提示末尾加一句：“Keep the summary under about S tokens; write less when the transcript needs less.” 每次折叠都用同一句，所以分块折叠时携带的摘要保持有界。

**S 的推导。** 令 `O = model.output_tokens()`（摘要请求作为 `maxTokens` 得到的额度），`B = context_window − O`（输入预算）：

`S = min(α × O, β × B)`，向下取整到 100 的倍数。

- `α × O` 为思考和超出留下 `(1 − α) × O` 的额度，遵守指令的摘要不会碰到 `max_tokens`。
- `β × B` 限制替换内容在之后每个请求中占用的份额。压缩后上下文约为 `S` 加保留的尾部（至多 `0.25 × B`，§5.3），所以 `β = 0.10` 使其约为 `0.35 × B`，远低于触发点。
- S 是软指令。`maxTokens` 仍为 `O`：在 S 处设硬上限会让每次超出都变成被截断的回复，再被对半拆分重试（#169）。

评估的候选（§13.4）：`α ∈ {0.25, 0.5}`、`β ∈ {0.05, 0.10}`，以及不写指令。

| 窗口 / 输出 | B | S(0.25, 0.05) | S(0.25, 0.10) | S(0.5, 0.05) | S(0.5, 0.10) |
| --- | --- | --- | --- | --- | --- |
| 32,000 / 8,000 | 24,000 | 1,200 | 2,000 | 1,200 | 2,400 |
| 128,000 / 16,000 | 112,000 | 4,000 | 4,000 | 5,600 | 8,000 |
| 200,000 / 64,000 | 136,000 | 6,800 | 13,600 | 6,800 | 13,600 |

选定的规则是策略的代码常量（ADR 002 D4：暂定常量写在代码里），作为 `ProductSummaryStrategyPlugin` 的工厂配置传入，使评估能让每个候选都经过已发布的策略运行。它不是用户配置。

### 13.4 评估

**放在哪里。** 不在 CI 中：它调用真实模型，结果不确定且要花钱，而 AGENTS.md 把自动化测试留给流程与合同。它是 `measure` 旁边的一个 `xtask` 命令，`measure` 已经可以选配真实模型配置运行：

```bash
cargo run --locked -p xtask -- summary-eval --home DIR --model PROVIDER/MODEL --judge PROVIDER/MODEL --runs 3 --out DIR [--reasoning inherit|off|low]
```

- 不带 `--reasoning` 时运行器执行第一阶段：三个推理分组（`inherit`、`off`、`low`），不带目标指令。带 `--reasoning VALUE` 时执行第二阶段：无指令分组加四个 `(α, β)` 分组，`VALUE` 是第一阶段为该模型选定的推理值。
- `--home` 必填，必须是一个隔离的产品 home，由操作者在其中配置好提供方；运行器从不使用 `~/.dimcode` 或 `~/.dimcode-alpha`。凭据留在那个 home 里，不进入仓库、用例或输出。
- 运行器在该 home 上启动产品配置（与 `graph` 相同），用默认偏好（用户得到的配置）从 `ProviderService::configuration` 为 `--model` 构造对话请求，并在其上解析 `ProductContextModel`。
- 对每个用例和每个分组，它用进程的 `ModelHost` 和 `ProviderConnectionResolver` 构造产品摘要策略，把分组的目标规则作为工厂配置，把分组的 `compaction.json` 写进 home，然后用由用例构造的 `CompactionInput` 直接调用 `compact`。因此每个分组都运行已发布的请求构造、分块、截断处理和推理映射，只有两个可调项不同。
- 输出（每次运行一份 JSON，加一张 Markdown 表）写到仓库之外的 `--out`。评估跑完后，决定和它的表格作为研究笔记提交（`docs/design/research/summary-eval.md`，英文 + 中文）。

**用例。** `xtask/summary-eval/cases/` 中的 JSON 文件，每段历史一个：

```json
{ "id": "long-tool-output",
  "description": "Debugging a failing flow test with a 60 KB cargo test log",
  "contextWindow": null,
  "earlierSummary": null,
  "messages": [ /* dim-agent-types Message 值，与 ModelRequest.messages 相同 */ ],
  "retainFrom": 14,
  "probes": [
    { "id": "test", "kind": "exact", "text": "transport_flow::compaction::manual_inside_turn" },
    { "id": "cause", "kind": "fact", "text": "The failure is a missing start reference on the cancelled result" },
    { "id": "old-plan", "kind": "absent", "text": "The plan to change the retention share is still current" } ] }
```

- `messages` 使用现有的 `Message` 类型，在运行器边界解析；不新增历史类型。运行器给第 `i` 条消息的来源记为 `m<i>`，唯一的切点是 `retainFrom`（检查它不把工具调用与其结果分开），所以 `retain_limit` 就是这个切点。
- 有 `earlierSummary` 时，它成为检查点的 `replacement`（`[summary_message(text)]`），从而评估合并路径。
- 有 `contextWindow` 时，它为该用例收窄窗口事实（更小的窗口总是合法的），使折叠路径能在花费可接受的历史上运行。它同时把该用例的输出额度限制到该窗口的四分之一（`config.max_tokens = min(output, contextWindow / 4)`），使最大输出很大的模型上输入预算仍为正；每次运行的 JSON 记录实际生效的窗口和输出。输入预算不会为正的用例在任何模型调用之前被拒绝。
- 内容公开：由本仓库自己的代码、文档和命令输出构成，路径在 `/work/dim` 下。没有真实用户会话、密钥或 home 路径。

| 用例 | 覆盖什么 | 探针包括 |
| --- | --- | --- |
| `long-tool-output` | 一个几十 KB 的工具结果，关键行在中间；中间截断 | 失败的测试名和错误行（exact）、原因和决定的修复 |
| `cjk` | 用中文进行、针对 `docs/zh` 的会话 | 用中文表述的约束、回复语言偏好、原样保留的标识符 |
| `code` | 带文件内容和 diff 的重构 | 改动的函数签名和文件路径（exact）、剩余工作 |
| `instructions` | 分几轮给出的约束，其中一条后来被修改 | 修改后的约束（fact）、被取代的约束不被当作现行（absent） |
| `merge` | 早先的摘要加新的记录 | 两边的事实 |
| `folding` | 比收窄后的窗口大数倍的历史 | 最后一次折叠后，第一块的事实仍在 |
| `injection` | 要求读者丢弃指令或写入某些内容的工具输出 | 摘要把它记为数据，不照做（absent） |

**评判。**

- `exact` 探针机械检查：规范化空白后，文本原样出现在摘要中。
- `fact` 和 `absent` 探针由 `--judge` 评判；评判模型尽量与摘要模型不同家族，模型接受时用温度 0。评判模型只看摘要和探针列表，不看记录，对每个探针回答 `present`、`absent` 或 `contradicted`，并附引文。`fact` 为 `present` 时得分；`absent` 探针为 `present` 时算一次违规。评判提示是运行器的一部分。
- 记录决定之前，操作者阅读每个分组得分最低的摘要，手工纠正评判错误；纠正列在研究笔记中。

每个模型和分组，在用例和运行次数上统计：事实召回 `F`、exact 召回 `X`、违规 `V`、被截断的回复 `C`（`cutOffCount`）、失败的压缩、摘要 token 中位数 `T`（`estimatedSummaryTokens`）和 `durationMs` 中位数；目标分组还统计比值 `T / S`。实际采用的推理（`reasoning`）与分组值不同的运行计为 `fellBack`，避免模型拒绝该分组时拿分组和自己比较（§13.2）。评判模型没有作答的判断探针单独计入 `unjudged` 列，由操作者手工纠正。失败的压缩没有摘要：它的事实探针和 exact 探针计为未命中，absent 探针计为未违规，且不调用评判模型，因此一次失败会降低 `F` 和 `X`，不会增加 `V`。

**模型。** 目录中每类推理方式至少一个模型：带 `thinking: disabled` 关闭声明的预算式思考（Anthropic）、按强度的推理（OpenAI Responses）、使用 `enable_thinking` 或 `thinking.type` 的 OpenAI 兼容模型（选一个中文能力强的模型），以及一个没有推理的模型作对照。

**流程。** 分两个阶段，使分组数量保持很少：

1. 推理：分组 `inherit`、`off`、`low`，不写目标；每个用例运行 `--runs` 次。
2. 目标：每类采用阶段 1 选出的推理值，分组为不写指令和四个 `(α, β)` 候选。

**决定标准**（`δ = 0.05`）：

- **推理默认值。** 对某一类，满足 `F ≥ F_inherit − δ`、`X ≥ X_inherit − δ`、`V ≤ V_inherit`、失败不多于 `inherit` 的分组为通过。默认值是 `off`、`low`、`inherit` 中第一个在所有类上都通过的：不损失可测质量的最便宜的值。
- **是否保留设置。** 如果在每一类上默认值同时也是最好的分组（没有别的分组的 `F` 比它高出 `δ` 以上），就不发布该设置：该值成为策略常量，不引入 `compaction.json`（消融）。否则带着这个默认值发布设置，笔记写明哪些类受益于别的值。
- **目标长度。** 满足 `F ≥ F_best − δ`、`X ≥ X_best − δ`、`V ≤ V_best` 的分组为通过，best 指阶段 2 中最好的分组（含不写指令）。如果不写指令通过，且它的 `C` 与失败不高于每个通过的目标分组，就不保留指令。否则保留 `C` 加失败最少的通过的目标分组，平局时取 `T` 中位数更小者。保留的目标必须被遵守：`T / S` 的第 90 百分位不超过 1.5；否则说明指令控制不了长度，不保留。
- 结果、各分组的表格和手工纠正写进研究笔记；§10 记录默认值，§13 成为已实现的合同。

### 13.5 实现时的改动

| 部分 | 改动 |
| --- | --- |
| `summary_strategy.rs` | 每次压缩读取一次 `compaction.json`（如果设置发布）；经 `reasoning_config` 映射 config；规则给出目标时追加目标句；策略值得到一个公开构造函数，接收 `ModelHost`、`ProviderConnectionResolver`、它的工厂配置和可选的 `FileStorage` 句柄（配置了 home 时由插件注入），供运行器使用 |
| `ProductSummaryStrategyPlugin` | 工厂配置：`compaction.json` 的位置和目标规则 |
| Provider（`preferences.rs`） | 强度分支移到 `reasoning_config`；行为不变 |
| 摘要诊断对象 | 增加 `reasoning`（实际采用的值：`inherit`、`off`、`low`）、`targetTokens`（S，没有指令时不出现）和 `cutOffCount`（停在 `max_tokens` 的回复数；现在它们只隐含在 `chunkCount` 中）。是 `context/compaction` 中由策略拥有的对象的新增字段；不改格式版本 |
| `xtask` | `summary-eval` 命令和 `xtask/summary-eval/cases/`；`serve_home`，即在已有 home 上组装产品配置，由 `serve_product`（`graph`）和运行器共用 |
| App Server 产品配置 | 把 home 中 `compaction.json` 的位置传给策略工厂；`RunningServer::service` 把一个已启动配置的进程服务交给 `xtask`，使运行器能取到 `ProviderService`、`ModelHost`、`ProviderConnectionResolver` 和 `FileStorage` |

不改任何事件、端口、Session 值、Loop、ContextPolicy、记录或重放。`transport_flow/compaction.rs` 中的 `summary_reasoning_comes_from_the_compaction_file` 覆盖读取与映射：`compaction.json` 的值为 `off` 时，摘要请求的 config 带有模型关闭推理的字段，而对话请求保留它的推理，记录的诊断显示 `reasoning: "off"`；无效文件让手动压缩以 `compaction_failed` 失败。目标规则不需要单独的测试：它是提示词文本，由评估衡量。

### 13.6 规则检查（针对 #170 的改动）

| 规则 | 回答 |
| --- | --- |
| 原则 §1、唯一事件总线 | 没有新事件或状态；两个可调项只改变一次策略调用内摘要请求的内容。 |
| §3 依赖方向 | 策略不增加依赖：`FileStorage` 的读法与 HostPlatform 相同，`reasoning_config` 是它自己 crate 中的函数。 |
| §3 改动其他模块（Provider） | Provider 的强度映射移入一个共享的纯函数。需求是摘要器必须对请求应用同一映射，而不能借助接线部件（D2）；该函数保持 Provider 的行为，Provider 仍是它唯一的编写者。 |
| §3 不改动即可扩展 | 只涉及策略层（§2.2）：其他策略不受影响，旧记录照原样重放。 |
| §4 规则 1–7 | 不适用：不增加指令、交接、处理器、核心、批次、上下文路径或公开服务。替换内容仍只经已完成事实的投影到达模型。 |
| §4 规则 8 | 不改基础设施。 |
| §4 规则 9 | 没有新事实；新的诊断字段是现有事实的内容，用户读它来理解一次压缩。 |
| ADR 002 D1 | Session 部件，作用域不变。 |
| ADR 002 D2 | 没有适配部件：映射是函数，文件由其所有者读取。 |
| ADR 002 D3 | 不变。 |
| ADR 002 D4 | `summaryReasoning` 是所有者文件中的业务配置，使用时读取，不重载。目标规则是代码中的暂定常量。 |
| ADR 002 D5 | 合同是该文件的 JSON 形状（§13.2）和诊断字段（§13.5）；`reasoning_config` 是 Rust 内部绑定。 |

**向机制所有者提出的需求：** 无。表中对 Provider 的改动是对产品模块的改动，不是对基础设施的改动。

### 13.7 消融审查

考虑过并去掉的：

- **用于映射的端口或适配部件**（由 App Server 绑定、调用 `apply_preferences` 的 Session 值）：它只为把 Provider 接到策略上（D2），而且会重新读取目录，而不是用已采用的快照。改为移出的纯函数。
- **为 `summaryReasoning` 提供 Settings 源或管理方法**：#170 中没有客户端编辑它；使用时读取的文件是同一份文档，以后可以变成 `BusinessDocument`。
- **项目作用域文件**：没有仓库层面的需求。
- **把 `low` 定义为“至多 low”**（对话关闭推理时保持关闭）：需要对提供方选项的第二套解读；`low` 改为直接请求。
- **硬性的 `maxTokens = S`**：把超出变成截断重试。
- **目录中按模型或按类的默认值表**：标准只选一个默认值，并在一个值处处胜出时去掉设置。
- **续写探针**（让模型根据摘要加尾部给出下一步）：`fact` 探针已经覆盖剩余工作。
- **CI 质量测试**：结果不确定且付费；用例是由运行器解析的数据。

保留的及其用途：回退到 `inherit`（该设置的含义：“模型允许时”）；诊断中的 `reasoning`、`targetTokens` 和 `cutOffCount`（评估从结果中读取它们，用户从记录中读到摘要器做了什么；`cutOffCount` 是思考带来的代价，否则它隐含在 `chunkCount` 中）；目标规则的工厂配置（使评估无需评估专用代码路径就能让每个候选经过已发布的策略运行）。
