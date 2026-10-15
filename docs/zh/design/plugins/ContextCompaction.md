# ContextCompaction

状态：草案（待评审）。路线图项 F1（[#72](https://github.com/arcships/dim/issues/72)）。
总体设计：[附录 B.1 ContextPreparation 行](../overview.md#b1-session-与分支-capability)。
作用域：[ADR 002](../../adr/002-plugin-scope.md)（两个 Session 部件，加上对现有分支 Capability
ContextPreparation 的补充）。本文内容均未实现。英文版：[ContextCompaction](../../../design/plugins/ContextCompaction.md)。

## 1. 概要

自动压缩已经存在：每次 `context/prepare`，ContextPreparation 的 ContextPolicy 估算输入、写
`context/budget`，超过阈值或预算恢复时调用 `ContextModel::compact` 并提交 `context/compaction`
（[context_policy.rs](../../../../crates/dim-agent-capabilities/src/context_policy.rs)）。生产环境里它从不运行，
因为 App Server 绑定的是 `ContextModelHost(None)`（[main.rs](../../../../crates/dim-app-server/src/main.rs)），
ContextPreparation 没有策略，Loop 也没有预算恢复。

F1 只加三样东西：

1. **生产宿主。** Session 部件 `ContextModel` 提供 `ContextModelHost(Some(..))`，用两个现有进程服务实现现有端口
   `ResolveContextModel` → `ContextModel`（[host.rs](../../../../crates/dim-agent-capabilities/src/host.rs)）：
   `ProviderConnectionResolver`（连接与凭据）和 `ModelHost`（Aimux，执行）。压缩用请求自己的模型；上下文窗口和输出预算来自请求上已采用的能力快照；token 计数是本地估算。
2. **手动压缩。** `session.context.compact` 在空闲分支上走同一条压缩路径，提交同样的 `context/compaction` 事实。
3. **用量查询。** `session.context.usage` 返回此刻策略会算出的预算：上下文窗口、输出预算、阈值和估算输入。不写任何东西。

两个方法由第二个 Session 部件 `ContextMethods` 经 `sessionMethods`（命名空间 `session.context`）提供，每次调用时找到对应分支的 ContextPreparation。

不新增事件、配置或进程插件；Provider、Aimux、Timeline、Dispatcher、事件格式和 rutis 都不改。

### 1.1 为什么这么小

- **不用 AuxiliaryModel。** S4 的 `auxiliaryModel.complete` 只返回文本。压缩需要被压缩上下文所属模型的上下文窗口、输出预算和 token 计数，那个调用不携带这些（[auxiliary-model §1.1](https://github.com/arcships/dim/blob/roadmap/s4-provider-purpose-policy-and-auxiliarymod/docs/design/appserver/auxiliary-model.md)）。
- **不放进 Provider。** 宿主需要的东西都能经 `dim-agent-capabilities::host` 的通用端口拿到：能力快照随 `ModelRequest` 传递；凭据由 `ProviderConnectionResolver`（产品配置下是 Provider 的 `ProductResolver`）解析，和 ModelProvider 每个 turn 的做法完全相同；执行用 `ModelHost`。把摘要和 token 估算放进 Provider，会让目录和连接的所有者多出一块与它无关的上下文策略逻辑。
- **不加压缩模型设置。** 用 Session 已采用的模型。单独的压缩模型是以后 ContextPreparation 自己的业务设置（§10），不在完成标准内。
- **不加新事实。** 手动压缩提交现有的 `context/compaction` 事实；现有的投影、检查点选择和快照对它原样适用。

## 2. 结构

```
 进程作用域（现有，不变）
 +-------------------------------------------------------------------+
 |  Aimux: ModelHost                                                 |
 |  Provider: ProviderConnectionResolver (ProductResolver)           |
 +-----------------^-------------------------------^-----------------+
                   | 注入（硬依赖）                | 注入（硬依赖）
 Session 作用域    |                               |
 +-----------------+-------------------------------+-----------------+
 |  ContextModel（新部件）                                           |
 |    提供 S:ContextModelHost(Some(ProductContext))                  |
 |                                                                   |
 |  ContextMethods（新部件）                                         |
 |    提供 sessionMethods["session.context"]: compact, usage         |
 |    每次调用: TreeCommands.get(branchId).service::<ContextPreparation>()
 +---------------------------------^---------------------------------+
        Rpc（C1）分发 -------------'          | S:ContextModelHost
 分支作用域                                   v （现有注入）
 +-------------------------------------------------------------------+
 |  ContextPreparation（现有，补充）                                 |
 |    自动路径: ContextPolicy.prepare（不变）                        |
 |    新增: compact()、usage()；由 turn/start、turn/end 折叠 turn 标记 |
 |    写 context/budget（仅自动）、context/compaction                |
 |  Loop（现有）: 宿主为 Some，预算恢复生效                          |
 +-------------------------------------------------------------------+
```

两个 Session 部件分开，是因为依赖不同：分支注入 `ContextModelHost`，它的提供者不能依赖由分支树提供的 `TreeCommands`。两者都放在 `dim-agent-capabilities`，紧挨 ContextPreparation。所有依赖都从新部件指向现有核心服务；没有现有部件引用它们。

## 3. 数据与状态

| 项 | 所有者 | 类别 |
| --- | --- | --- |
| `reason: manual` 的 `context/compaction` | ContextPreparation | 现有事实，现有格式 |
| turn 标记：已消费的 `turn/start` 到其 `turn/end` 之间的 turn id | ContextPreparation | 私有内存状态，由已消费事件折叠，装载时重建（它已经为中断标记折叠这些事件） |
| 正在运行的手动压缩（取消令牌） | ContextPreparation | 自有任务的 I/O 状态；没有事件打开它，重载时无需结算 |

宿主不持有状态。`ProductContext` 持有 `Arc<ModelHost>` 与 `Arc<ProviderConnectionResolver>`；它为一个请求返回的 `ContextModel` 持有该请求。解析出的连接只存在于一次 `compact` 调用内。

## 4. 接口

### 4.1 语言中立（ADR 002 D5）

`sessionMethods`，Session 作用域，键 `session.context`，由 ContextMethods 提供
（[session-boundary A.1](../contracts/session-boundary.md#a1-方法贡献)）。两个方法都是 `interruption: report`、`progress: false`、`notifications: []`、`reply: direct`。

| 方法 | 变更 | 请求 | 回复 |
| --- | --- | --- | --- |
| `session.context.compact` | 是 | `{sessionId, branchId}` | `{status: "completed", sequence, estimatedInputTokens, compactedInputTokens}` 或 `{status: "skipped", reason: "nothing_to_compact"}` |
| `session.context.usage` | 否 | `{sessionId, branchId}` | `{through, contextWindow, outputTokens, thresholdTokens, estimatedInputTokens, baselineSource: "provider_baseline" \| "full_estimate"}` |

- `sequence` 是已提交的 `context/compaction` 事件。`estimatedInputTokens` 是策略在压缩前的估算；`compactedInputTokens` 是策略验收过的压缩后请求的 `count_tokens`。
- `through` 是计算用量时的已提交头部。数值等于在该头部做一次准备时写进 `context/budget` 的值，外加 `thresholdTokens`。
- 取消：按 session-boundary A.1（卸载会中断，调用方断开不取消任何东西）。手动压缩还会被分支自己的事实取消（§5.3）。
- 错误是 `MethodError` 数据（§6）。

`ContextModel` 端口是 Rust 运行时内部现有的宿主端口，不是插件合同；它和其他类型化宿主端口一样保持 Rust trait（ADR 002 D5：按名暴露是 #4 的工作）。

### 4.2 Rust 绑定

```rust
// dim-agent-capabilities/src/context_model.rs（新）
pub struct ContextModelPlugin { scope: SessionScope }
impl TypedPlugin for ContextModelPlugin {
    type Deps = (Arc<ModelHost>, Arc<ProviderConnectionResolver>);   // 进程作用域
    // apply: ctx.provide_as(scope.key::<ContextModelHost>(),
    //          Arc::new(ContextModelHost(Some(Arc::new(ProductContext { host, resolver })))))
}
struct ProductContext { host: Arc<ModelHost>, resolver: Arc<ProviderConnectionResolver> }
impl ResolveContextModel for ProductContext { /* §5.1 */ }

// dim-agent-capabilities/src/context_methods.rs（新）
pub struct ContextMethodsPlugin { scope: SessionScope }
impl TypedPlugin for ContextMethodsPlugin {
    type Deps = InSession<TreeCommands>;
    // apply: SessionMethods::provide(ctx, &scope, "session.context", decls, handler)
}

// dim-agent-capabilities/src/host.rs（修改同一 crate 的现有端口）
pub struct CompactionInput { /* 现有字段 */ pub trigger: CompactionTrigger }

// dim-agent-capabilities/src/context_preparation.rs（补充）
impl ContextPreparation {
    pub async fn compact(&self, cancel: CancellationToken) -> Result<CompactOutcome, ContextOpError>;
    pub async fn usage(&self, cancel: CancellationToken) -> Result<ContextUsage, ContextOpError>;
}
impl PublicService for ContextPreparation {}   // lib.rs，同 Approval、Permission

// dim-app-server/src/main.rs，仅产品配置（没有 --models 文件）：
builder
    .session_plugin(ContextModelPlugin::new)
    .session_plugin(ContextMethodsPlugin::new)
    .session_methods("session.context");
// 在这里替换 .session_value(Arc::new(ContextModelHost(None)))；--models 与 mock 配置
// 仍是 ContextModelHost(None)
```

`SessionMethods`、`MethodDecl`、`Handler` 是 C1 放在 `dim-agent-capabilities::methods` 的类型。
`CompactionInput.trigger` 由 ContextPolicy 设置（`Threshold`；有 `provider_budget` 时为 `ProviderOutputBudgetExhausted`；`Manual`），宿主据此填写诊断里的 `reason` 和 `trigger`，而不是由 ContextPreparation 改写宿主的结果。

## 5. 流程

### 5.1 生产宿主

`ProductContext::resolve(request)`：

1. `request.capability_snapshot` 必须存在且对请求的模型有效，否则返回 `InvalidRequest`“Model capabilities are not adopted; select the model again”（与 `ProductResolver` 的条件和文本相同，那里本来就会拒绝这种请求）。
2. 返回绑定该请求的 `ContextModel`：

| 方法 | 值 |
| --- | --- |
| `context_window` | `snapshot.capabilities.contextWindow`（目录值，含目录现有的覆盖） |
| `output_tokens` | 有 `request.config.maxTokens` 时取它，否则取 `snapshot.capabilities.maxOutputTokens`，并以后者为上限 |
| `threshold_tokens` | `0.8 × (context_window − output_tokens)`（取值见 §10） |
| `count_tokens` | 本地估算：序列化后的系统提示、消息和工具声明的 UTF-8 字节数除以 4，每个图片部件按固定 1,600 token 计，而不按编码字节计（取值见 §10）。策略只在以下场合用它：在提供方报告的 `prompt_tokens` 之上计算增量、第一个请求、`validate`、以及检查压缩是否减少了输入 |
| `compact` | 见下 |

`compact(input, cancel)`：

```
 ContextPolicy         ContextModel（宿主）        ProviderConnectionResolver   ModelHost (Aimux)
   | compact(input) ------>|                                 |                       |
   |                       | cursor = input.history 中最后一个合法切点（与           |
   |                       |   ContextPolicy 的切点检查规则相同）；没有 -> Skipped    |
   |                       | 分块 = 先前摘要 + entries[..=cursor] 的文本，            |
   |                       |   每次调用不超过 窗口 - 输出 - 提示                      |
   |                       | 对每一块：                                               |
   |                       |   request = {同一 provider/model/快照/config，           |
   |                       |     maxTokens = output_tokens，system = 摘要提示，       |
   |                       |     一条 user 消息，不带工具}                            |
   |                       |   resolve(request) ----------->|                       |
   |                       |   <-- ResolvedModelConnection（请求内局部）              |
   |                       |   invoke_with_recovery(...) ------------------------->|
   |                       |   <-------------------------------- 文本、MessageStop |
   |                       |   当前摘要 = 文本                                        |
   |<-- Completed {cursor, summary, diagnostics} | Skipped | Err(HostError)          |
```

- **模型。** 请求的 provider、model、能力快照和 config。丢弃 thinking 输出，只有文本构成摘要。工具停止或空文本算失败。
- **摘要范围。** 到 ContextPolicy 切点检查接受的最后一个切点为止的全部内容（不保留被移除调用的工具结果，下一条不是工具上下文），包括 `input.checkpoint` 里的先前摘要。按键的基线块不进摘要：策略照旧从切点处的权威基线重放它们。
- **记录文本。** 每条按角色标注成文本；工具调用写名称和输入，工具结果写其文本，图片写 `[image]`。单条就超过一次调用输入预算的，从中间截断并加标记，计入 `compactedToolPayloadCount`。
- **分块。** 一次调用放不下时按顺序折叠：每次调用拿到当前摘要和下一块。`chunkCount` 与 `modelCallCount` 记录次数。预算恢复时历史可能超过窗口，也由此覆盖。
- **摘要提示。** 本模块拥有的固定文本：保留用户的目标、约束和最新请求、已作出的决定、涉及的文件和命令、当前状态和待办的下一步；只输出摘要。
- **失败策略。** `invoke_with_recovery`（`dim-agent-capabilities::model_provider`），即 ModelProvider 使用的函数：共享预算内的瞬时退避、一次凭据恢复、终态拒绝的结算、`forbids_retry`。S4 抽出了它；如果 F1 先合入，由 F1 做同样的抽取。
- **诊断。** `CompactionDiagnosticsRecord` 的必填字段由本次运行填写（`reason`/`trigger` 取自 `input.trigger`，`coverage: full`，各计数、估算和 `durationMs`）；宿主不计算的可选字段省略。

随后 ContextPolicy 照现在的做法处理：验收切点，并确认压缩后的请求更小且低于窗口，提交 `context/compaction`，保存可选快照。

### 5.2 自动压缩

代码路径不变；因为 ContextPreparation 现在有了策略，它开始生效。宿主绑定后，Loop 的预算恢复也生效：`validate` 发现估算达到或超过窗口时，请求以预算事实结束，Loop 带 `recovery` 重试一次，强制压缩（`trigger: provider_output_budget_exhausted`）。

### 5.3 手动压缩

```
 客户端     Rpc（C1）    ContextMethods      ContextPreparation（分支）          Timeline
   | session.context.compact {sessionId, branchId}
   |---------->|---------->| TreeCommands.get(branchId)
   |           |           |   .service::<ContextPreparation>().compact(cancel)
   |           |           |------------------>| 状态锁：已关闭？turn 进行中？
   |           |           |                   |   手动压缩已在运行？
   |           |           |                   |   -> context_busy
   |           |           |                   | 登记任务（受跟踪），标记运行中
   |           |           |                   | head = committed；context(head)；渲染
   |           |           |                   | request = 最新 context/configured 的
   |           |           |                   |   设置 + system + 投影后的消息
   |           |           |                   |   （没有 -> context_not_configured）
   |           |           |                   | 策略：resolve、估算、
   |           |           |                   |   compact(trigger: Manual)、验收
   |           |           |                   | Completed -> 追加 context/compaction --->|
   |           |           |                   |   {turnId = requestId = 操作 id,
   |           |           |                   |    through = head}；保存快照
   |<---------------------------------------- {status, sequence, ...}
```

- 请求按 Loop 的方式由固定上下文构造：最新 `context/configured` 的设置（provider、model、快照、工具、config、metadata）和 system，加上投影后的历史（`context_messages`）。不会发出带工具的模型请求，所以不绑定工具。
- ContextPolicy 的压缩步骤（调用 `compact`、验收、追加、快照）由自动和手动路径共用；手动路径不写 `context/budget`（它不是一次准备），只提交 `Completed` 结果。`Skipped` 结果就是 `skipped` 回复；失败是 `compaction_failed` 回复。两者都不写事实：后续决定不读它们（规则 9），指令没有生效就不写结果（规则 1）。
- 事实的 `turnId` 与 `requestId` 填同一个新的操作 id（UUID）。没有 `turn/start` 与之对应：ContextPolicy 的重试过滤只匹配它自己的 turn，投影取最新的已完成压缩而不看 id。Trace 把该事件作为单独的操作分组。
- 取消来自分支自己已消费的事实：运行期间消费到 `stop/interrupt`（与正在运行的准备一样）或 `turn/start`，就以 `compaction_cancelled` 取消。宿主已经返回的压缩照样提交，与自动路径相同。ContextPreparation 或方法提供者卸载会中断它（`operation_interrupted`）；ContextPreparation 的清理像等待准备任务一样等待它。

### 5.4 用量查询

`usage(cancel)` 读取已提交头部，按 §5.3 构造请求，解析 `ContextModel`，在上下文事件上运行 ContextPolicy 现有的估算（最近的 `model/usage` 与请求前缀匹配时用提供方基线加增量，否则完整估算）。可以在 turn 进行中调用。不加锁、不启动任务、不写任何东西。

## 6. 错误

`MethodError` 代码（未注明的为 `code -32000`）：

| 代码 | 方法 | 何时 | 重试 |
| --- | --- | --- | --- |
| `-32602` | 两者 | 缺少 `branchId` 或不是字符串 | 否 |
| `session_invalid_input` | 两者 | 未知分支（`AgentError::UnknownBranch` 的现有映射） | 否 |
| `session_closed` | 两者 | Session 或分支正在关闭 | 否 |
| `context_not_configured` | 两者 | 分支历史中没有 `context/configured` | 采用模型之后 |
| `context_model_failed` | 两者 | `resolve` 失败（没有能力快照） | 重新选择模型之后 |
| `context_busy` | compact | turn 进行中（准备只在 turn 内运行），或另一次手动压缩在运行 | 空闲时 |
| `compaction_failed` | compact | 共享失败策略之后的宿主错误、空摘要、非法切点，或结果没有把输入降到窗口以下；消息为 `HostError` 文本 | 由用户决定 |
| `compaction_cancelled` | compact | 运行期间消费到 `stop/interrupt` 或 `turn/start` | 由用户决定 |
| `operation_interrupted` | compact | ContextPreparation 或 ContextMethods 卸载（session-boundary A.1，`report`） | 由用户决定 |

自动压缩的错误处理不变：宿主错误成为事实中的 `Failed` 结果，请求不压缩继续；在恢复中则使请求失败。

## 7. 安全

- **凭据只在请求内。** 宿主在一次 `compact` 调用内从 `ProviderConnectionResolver` 取得 `ResolvedModelConnection` 并交给 `ModelHost`，与 ModelProvider 对 turn 的做法完全相同。它从不保存、返回、序列化或记录连接。Provider 不变，仍是唯一读取凭据的地方。
- **没有新的数据外发。** 摘要调用把 Session 自己的历史发给每个 turn 都会收到它的同一个提供方和模型。
- **摘要是不可信的模型输出。** 它只经现有的 `summary_message` 进入上下文，被标为数据并转义。
- **结果中没有密钥。** 事实、回复和诊断只含计数、id 和 `HostError` 消息，后者不含凭据值；请求和摘要内容不进 tracing。
- **授权。** Rpc 对 Session 方法校验 `sessionId`（C1）。每个能调用 Session 方法的连接都可以压缩，正如它可以提交输入。

## 8. 与现有部件的关系

| 部件 | 关系 |
| --- | --- |
| ContextPolicy | 自动逻辑不变；压缩步骤与手动路径共用；设置 `CompactionInput.trigger` |
| ContextPreparation | 补充：`compact`、`usage`、turn 标记、`PublicService` 标记。准备、steer、中断和重载处理不变 |
| Loop | 代码不变；产品配置下 `ContextModelHost` 为 `Some`，预算恢复生效 |
| ModelProvider | 代码不变；产品配置下 `validate` 开始按估算检查窗口（估算达到窗口的请求在发起网络调用前以预算事实结束，由 Loop 恢复） |
| Provider、ProductResolver | 不变；只经 `ProviderConnectionResolver` 使用 |
| Aimux / ModelHost | 不变 |
| AuxiliaryModel（S4） | 不使用。共用 `invoke_with_recovery`；S4 与 F1 谁先合入谁负责抽取 |
| `sessionMethods`（C1） | 两个方法需要它。宿主部件不需要 C1，可以先合入 |
| HistoryRenderer、快照、原生存储 | 不变；手动事实与自动事实一样被投影和做快照 |
| Trace | 手动压缩显示为单独的操作 id |
| `--models` 配置（ConfiguredModels） | 保持 `ContextModelHost(None)`：其请求没有能力快照，没有可据以压缩的上下文窗口（§10） |
| mock 配置、测试 | 保持 `ContextModelHost(None)` 或各自的测试宿主 |
| ManagedProviders 套餐用量 | 不属于 F1；该补充存在后 `/usage` 可以与本回复并列显示 |
| CLI `/compact`、`/usage` | 调用这两个方法；渲染是 CLI 的工作 |

## 9. 规则检查

| 规则 | 回答 |
| --- | --- |
| 原则 §1（Capability、App Server 边界） | 两个 Session Capability 放在 `dim-agent-capabilities`；App Server 只在产品配置中安装它们。 |
| §1 唯一事件总线 | 只使用分支 Timeline 和 Dispatcher；没有第二条总线，没有新的订阅机制。 |
| §3 依赖方向 | 新部件依赖核心服务（`ModelHost`、`ProviderConnectionResolver`、`TreeCommands`、ContextPreparation）；现有部件只经现有的 `ContextModelHost` 键用到新部件，而安装该部件的配置总会提供这个键。没有可选依赖。 |
| §4 规则 1（指令事件） | 客户端经所有者的方法请求，所有者自己决定并写自己的事实，符合 session-boundary 的规定（“改变分支状态的方法调用其所有者，由所有者写入”）。没有生效就没有事实。 |
| §4 规则 2（交接） | 不涉及；没有对象换所有者。 |
| §4 规则 3（before 回调） | 不使用。 |
| §4 规则 4（核心无业务含义） | Timeline、Dispatcher 和运行时门面不变。 |
| §4 规则 5（原子批次） | 手动事实是单个事件。 |
| §4 规则 6（上下文只来自交接后的事件） | 摘要只经已完成压缩事实的现有投影到达模型；没有新的上下文路径（AGENTS 规则 10）。 |
| §4 规则 7（公开事件与服务） | `TreeCommands`、`PublicService` 上的 `Branch::service`、`ProviderConnectionResolver`、`ModelHost`、`sessionMethods`。 |
| §4 规则 8（基础设施不变） | Timeline、Dispatcher、事件信封与格式、rutis、loader、Agent 装配、Session 与分支门面都不改。不向 rutis 提需求。 |
| §4 规则 9（只写需要的事实） | 只写已完成的手动压缩，因为后续投影要用；用量查询不写任何东西。 |
| ADR 002 D1（作用域） | 宿主与方法：Session（各分支共用，无跨 Session 状态）；操作与 turn 标记：分支，放在压缩事实的所有者中。 |
| ADR 002 D2（无接线模块） | ContextMethods 经规定的 `sessionMethods` 暴露所有者的操作；ContextModel 是本功能的宿主。两者都不用于连接其他插件。 |
| ADR 002 D3（依赖） | 只硬依赖核心；Provider 或 Aimux 重载会像今天传播到 ModelProvider 一样传播到这些部件（已接受的代价）。 |
| ADR 002 D4（配置） | 不增加配置；常量写在代码里。 |
| ADR 002 D5（语言中立合同） | §4.1 是方法合同；§4.2 是一种绑定。`ContextModelHost` 是现有的类型化运行时服务，不变。 |

**影响面（新增公开内容）：** `session.context` 命名空间及两个方法（`compact`、`usage`），`/compact` 和 `/usage` 需要它们，现有合同不提供。内部 Rust 改动：`CompactionInput.trigger`（同一 crate 的现有端口，让生产者说明触发原因）和 ContextPreparation 上的 `PublicService`（让 Session 部件按分支找到它）。

**向机制所有者提出的需求：** 无。

## 10. 待定问题

1. **常量。** 阈值 `0.8 × (窗口 − 输出)`、估算 `字节数 / 4` 和每张图片 1,600 token 是提议值。该估算对部分文字和分词器偏低；第一个请求之后由提供方基线纠正。这些值可以接受吗？以后是否要用提供方的分词器？
2. **保留的后缀。** 方案摘要到最后一个合法切点为止的全部内容，包括最新的用户请求（与现有示例宿主一致）。是否改为原样保留最近一条用户输入，代价是多一条保留预算规则？
3. **手动事实的 id。** 在 `turnId` 和 `requestId` 中填新的 UUID，格式不变。这样可以接受吗？还是手动压缩应在一个 turn 内运行，那需要 Loop 能在没有输入时开启 turn？
4. **`--models` 配置。** 它没有能力快照，因此没有压缩。接受，还是给 ConfiguredModels 加上下文窗口（F1 之外的配置改动）？
5. **单独的压缩模型。** 推迟。如果需要，它是 ContextPreparation 自己的业务设置，经现有的 `ModelSelectionHost` 解析出能力快照。
6. **提供方报告的溢出。** Aimux 报告提供方的上下文长度错误时不带预算事实，所以只有本地估算会触发恢复。把提供方溢出错误映射为预算事实是 Aimux 的问题，不在 F1 内。

## 11. 验收（流程测试）

产品配置的 App Server，连接一个 mock HTTP 提供方，其目录条目的上下文窗口很小（`crates/dim-app-server/tests`）：

1. **自动。** 对话增长超过阈值；下一次准备写 `context/budget`，发出一个摘要请求（摘要系统提示、一条 user 消息、不带工具、Session 的模型），提交 `completed` 的 `context/compaction`，`reason: auto, trigger: threshold`；该 turn 的模型请求以摘要开头，不再包含被摘要的消息。
2. **预算恢复。** 估算达到窗口的请求以预算事实结束，Loop 重试一次，重试时以 `trigger: provider_output_budget_exhausted` 压缩。
3. **手动。** 在空闲分支上调用 `session.context.compact`，回复 `completed` 及已提交的序号；事实为 `reason: manual`；下一个 turn 的请求以摘要开头。turn 进行中回复 `context_busy`；手动压缩期间 `session.stop` 回复 `compaction_cancelled`，不写事实。
4. **用量。** `session.context.usage` 返回目录窗口、输出预算、阈值和估算；压缩之后估算下降；调用后分支 Timeline 没有新事件。
5. **失败。** 提供方让摘要请求失败：自动路径提交 `failed` 结果，turn 不压缩继续；手动回复 `compaction_failed`，不写事实。
6. **凭据。** 恢复后的凭据再次被拒的托管账号只结算一次；事实、回复和诊断中不出现 API key。
