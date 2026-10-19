# 请求增量存储（原生格式 v17）

状态：提议，待团队评审。[#193](https://github.com/arcships/dim/issues/193) 的第 2 阶段，按该 issue 最新评论中商定的方向。第 1 阶段是在一次读取的各页之间共用有界内容缓存，已以 [#213](https://github.com/arcships/dim/pull/213) 合入；第 3 阶段是另一份运行时提议（§15）。负责线：Agents/存储（yyhhyyyyyy）。评审：zerob13、eric8810。范围：原生 `TimelineStore` 实现内部（`SqliteTimelineStore`，[timeline.rs](../../../../crates/dim-app-server/src/storage/timeline.rs) 与 [codec.rs](../../../../crates/dim-app-server/src/storage/codec.rs)），以及本设计将其升级到 v17 的[存储格式](../../../design/appserver/native-storage-format.md)。本文只是设计；实现在评审之后进行。

## 1. 概要

v16 格式中，`context/prepare` 或 `model/call` 行为请求的每条消息保存一个内容哈希引用。每个请求重复上一个请求再追加几条消息，所以第 k 次模型调用保存 k 个引用。存储、写入时的哈希和读取时的解码，都随两次压缩之间模型调用次数的平方增长（§2）。

- **规则。** v17 的请求行把请求保存为相对分支逻辑历史中上一个请求的结构增量：`base` 位置、`keep`（公共消息前缀的长度）、其余消息内联，`system` 与 `tools` 为“同 base”或内联。没有更早请求、或与它没有任何公共消息的请求是根，全部内联（§5）。
- **确定性。** 字节是分支逻辑历史与该事件的函数。每分支一份的写入方记忆只省去回读 base；有没有它，写入方写出的字节都相同（§6）。
- **读取。** 读取按序解码请求，每个请求由前一个请求得到。从历史中间开始的读取用一次递归主键查询取回 base 链（§7）。base 损坏或缺失时读取失败；请求绝不会被还原为空（§8）。
- **测量。** 在 mock、工具与产品窗口三种配置下，压缩之外的每个请求都延续上一个请求，且写入下一个请求时上一个请求总还在内存中（§4）。约 10,000 条事件时，v16 每条事件存储 13,778 字节，v17 将为 351 字节，与 1,000 条事件时相同（普通 mock 轮次；§12）。
- **格式。** v17 是在现有准入事务内把标记从 v16 原地改为 17，不改表、不改写任何行。v16 行永久可读，v16 与 v17 行可在同一数据库中混存，v17 写入方不再新增内容行（§11）。

事件类型及其冻结样本、Timeline、Dispatcher、`TimelineStore` trait、读取 API、能力和外部协议都不变。不新增事件、服务、方法、wire 字段、配置或运行时需求。

## 2. 问题与依据

### 2.1 恢复耗时（#193）

release 构建，Apple M4，一次性数据库上的 `SqliteTimelineStore`，`defaults::mock_hosts` 与 `defaults::install`。历史经 mock 模型轮次增长（普通轮写 10 条事件，工具轮写 18 条）；每个规模恢复 7 次。计时：`Agent::restore_session` 加第一次 `idle("main")`。

| 事件数 | 普通轮，中位数 | 工具轮，中位数 |
| --- | --- | --- |
| ~1,000 | 65 ms | 80 ms |
| ~3,000 | 327 ms | 404 ms |
| ~10,000 | 2.77 s | 3.48 s |

10,017 条普通事件时，2,772 ms 中有 2,186 ms 花在 11 次全历史读取（每次约 199 ms），549 ms 花在 `validate_session`。每个请求行为每条消息保存一个引用（`encode_refs`），所以 10k 条事件时这些行共有 2,000,000 个引用、137.5 MB 事件行，而不同内容只有 2,001 条；解码要经 `ContentCache` 解析每个引用。

### 2.2 第 1 阶段（#213）

第 1 阶段用一次 `foldhash` 查找解析一个引用，并让一次读取的各页、一轮 `validate_session` 共用一个缓存，缓存正文上限 64 MiB。写入方与写出的字节不变。其结果（两轮交错测量的平均值，每轮取 7 次恢复的中位数）：

| 组合 | 事件数 | 恢复加首次 idle | 每次读取 | `validate_session` |
| --- | --- | --- | --- | --- |
| 普通 | ~1,140 | 71 → 58 ms | 5.0 → 4.3 ms | 9.7 → 5.3 ms |
| 普通 | ~3,140 | 344 → 245 ms | 25.2 → 19.6 ms | 58.6 → 22.1 ms |
| 普通 | ~10,140 | 2,866 → 1,779 ms | 208 → 146 ms | 576 → 149 ms |
| 工具 | ~1,140 | 80 → 71 ms | 5.7 → 5.3 ms | 11.5 → 6.5 ms |
| 工具 | ~3,130 | 417 → 303 ms | 30.5 → 24.3 ms | 73.3 → 27.8 ms |
| 工具 | ~10,140 | 3,624 → 2,256 ms | 257 → 184 ms | 724 → 186 ms |

### 2.3 剩下的问题

把第 1 阶段普通轮的每次读取耗时拟合为 `a·E + b·R`（`E` 为事件数，`R` 为引用数，`T` 个普通轮时为 `2T² + 4T`），得到每条事件 a ≈ 2.9 µs、每个引用 b ≈ 58 ns；该拟合复现了 1,140 条事件的读取（4.5 ms，实测 4.3 ms）。10k 条事件时，每次读取的 146 ms 中有 117 ms 花在引用上。只有改格式才能去掉它们。

写入同样付出代价。每个请求行，v16 写入方序列化每条消息、tools 与 system prompt 并计算 SHA-256，再为每份内容执行一次 `INSERT OR IGNORE`。在 10k 条事件的普通 Session 的最后 200 个请求行上，SQL 之前每行要 539 µs，v17 编码为 24 µs（§6.3）。每个模型步骤还读取两次上下文历史（[timeline-storage §4](../../../architecture/rust-agent/timeline-storage.md#4-读取)），没有压缩时，这两次读取要解码此前所有请求。

存储建成时的检查使用频繁压缩，请求列表一直很短。两次压缩之间有大量模型调用的情况不在那次检查的范围内：mock 与 `--models` 配置从不压缩，而窗口较大时，基于 token 的压缩可能相隔数百步。

## 3. 约束

| 约束 | 出处 | 本设计 |
| --- | --- | --- |
| 事件以原类型写入与读出；编码只存在于存储内部 | [timeline-storage §2](../../../architecture/rust-agent/timeline-storage.md#2-存储编码) | 增量是存储内部对 `ModelRequest` 的编码；读取返回完整请求 |
| 请求留在事件里：`model/call` 携带交给模型的最终请求 | [trace.md](../../../architecture/rust-agent/trace.md#本地归档)（“`model/call` 中的 request 是交给模型能力的最终请求”） | 请求既不推导，也不延迟加载 |
| ContextPolicy 读取最近一个精确请求 | [context_policy.rs](../../../../crates/dim-agent-capabilities/src/context_policy.rs) 的 `ContextPolicy::estimate` 把 provider、model、能力快照、tools、system、config 与 metadata 同上一个 `model/call` 比较，并要求 `messages.starts_with` 其消息 | 解码出的请求与写入的相等 |
| 事件类型与冻结样本 `crates/dim-agent-types/tests/fixtures/events.json`、`events.ts` 及 `*_contract.rs` 测试 | [设计规则 §6](../../../architecture/rust-agent/principles.md#6-验证)，AGENTS.md 规则 6 | 不动 |
| Timeline（序号、原子批次、提交与发布、before）、Dispatcher（只有热窗口）、`TimelineStore` trait、读取 API、能力 | [timeline-storage §1、§3、§4](../../../architecture/rust-agent/timeline-storage.md) | 不动：端口保持签名与语义 |
| 外部协议仍展开为原事件 | [timeline-storage §4](../../../architecture/rust-agent/timeline-storage.md#有界读取与恢复扫描) | 不动 |
| 不兼容变化需要新版本与读取路径；升级只增加结构，不读取或改写任何业务行；单向 | [存储格式“格式版本”](../../../design/appserver/native-storage-format.md#格式版本)及 S5a/S5b 的决定 | 以标记升级到 v17（§11） |
| 序列化优化须测量正文复制、编码次数及耗时，不为优化增加业务状态 | [timeline-storage §4](../../../architecture/rust-agent/timeline-storage.md#有界读取与恢复扫描) | 测量见 §4、§6.3、§7.4 与 §12；写入方记忆不含业务状态，也不决定任何字节（§6.2） |
| 空闲分支的存活字节不随步数增长 | [timeline-storage §7](../../../architecture/rust-agent/timeline-storage.md#7-验收) | 记忆不持有请求；一次读取在返回内容之外只多持有一个已解码请求（§6.2、§7.1） |

## 4. 前缀稳定性

### 4.1 为什么相邻请求互相延续

只有每个请求以前一个请求开头，增量才小。三项现有合同保证两次压缩之间如此：

- **投影只追加。** 事件只经已采用的输入与工具结果进入模型上下文（AGENTS.md 规则 10，[设计规则 §4](../../../architecture/rust-agent/principles.md#4-通过时间线扩展行为) 第 6 条），历史渲染把它们放在末尾。`context/prepare` 的请求就是其 `through` 处渲染出的历史。
- **准备只能追加。** ContextPreparation 拒绝不以渲染历史开头的准备结果（“Context preparation must preserve the timeline history prefix; additional messages may be appended”，[context_preparation.rs](../../../../crates/dim-agent-capabilities/src/context_preparation.rs)）。因此除非 ContextPolicy 在中间压缩，`model/call` 的请求延续其 `context/prepare`。
- **ContextPolicy 已依赖这一点。** 它的 provider 基线只在 `request.messages` 以上一个 `model/call` 的消息开头时适用。

完成的压缩替换列表开头（replacement、基线、保留的尾部），所以压缩后的那个请求与其 base 没有公共前缀，是根（§10）。`system` 或 `tools` 变化（prompt 修订、工具采用）时，该部分内联一次。

### 4.2 测量

一个临时探针（不提交）在一次性数据库上用转发的 `TimelineStore` 包住 `SqliteTimelineStore`。每次 `append` 时，它把每个请求与同一分支上的上一个请求比较：先 `Arc::ptr_eq` 再 `==` 求公共前缀，比较 `system` 与 `tools`，并检查上一个请求的 `Arc` 是否仍然存活，也就是 `Weak` 记忆会看到的情况。它还构建精确的 v17 增量 JSON。release 构建，Apple M4。配置：

- **普通**与**工具**：`defaults::mock_hosts` 与 `defaults::install`，使用 [`restore_cost`](../../../../crates/dim-app-server/examples/restore_cost.rs) 的轮次；不压缩。
- **产品窗口**：产品窗口事实（`ProductContextModel`）与摘要策略（`ProductSummaryStrategyPlugin`）、`IdentityContext`、`HistoryRenderer::default()`、`MockTools`，以及窗口 32,000 token、输出 2,000 token 的能力快照。模型驱动像 `EchoModel` 一样回显，对摘要请求回答一段每次不同的简短摘要。轮次在普通文本与 `tool:echo` 之间交替，每轮带 1,000 字节，所以窗口会填满，约每 20 轮压缩一次。

统计每一个请求行：

| 配置 | 事件数 | 请求行 | 根（完成的压缩） | 保留 base 全长的行 | 保留消息 / 消息总数 | 每行内联消息数 | 追加时上一个请求仍存活 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 普通 | 1,001 / 3,001 / 10,001 | 200 / 600 / 2,000 | 1（0） | 100% | 99.0% / 99.7% / 99.9% | 1.00 | 100% |
| 工具 | 1,009 / 3,007 / 10,009 | 224 / 668 / 2,224 | 1（0） | 100% | 99.1% / 99.7% / 99.9% | 1.00 | 100% |
| 产品窗口 | 3,007 / 10,017 | 578 / 1,926 | 10（9）/ 33（32） | 98.4% / 98.3% | 97.4% / 97.4% | 1.45 / 1.48 | 100% |

“保留 base 全长的行”统计除第一个请求外的每个请求行；产品窗口中其余的恰好是完成压缩后紧接的 9 行和 32 行。按行看，`model/call` 不内联消息，`context/prepare` 内联两条（模型输出与下一条输入或工具结果），第一个请求内联一条，该配置中压缩后的根内联十条。

- 除压缩之后紧接的请求外，每个请求都延续其 base：`keep` 等于 base 的长度。每个根要么是 Session 的第一个请求，要么紧跟一次完成的压缩。
- `model/call` 保留其 `context/prepare` 的消息 `Arc`，所以它的前缀按指针找到。`context/prepare` 与上一个 `model/call` 没有共享指针：渲染每一步都构建新消息，所以它的前缀按 `==` 找到。值比较计入 §6.3 的 v17 写入成本。
- `system` 与 `tools` 只在根中内联。
- 每次追加请求时（包括 Session 空闲后新一轮的第一个请求），同一分支上的上一个请求都仍然存活。探针观察的是存活与否，不是由谁持有；ContextPreparation 与 ContextPolicy 各自保留最近一个已准备的请求，Timeline 也在热窗口中保留近期事件。因此运行中的 Session 里，`Weak` 记忆总能命中。
- 没有一次运行在 before 轨迹中出现请求。

### 4.3 未测量的部分

- 真实 provider 适配器（`ModelProviderPlugin::process`、aimux）与非恒等的准备宿主：这些配置使用 `IdentityContext` 和回显驱动。准备规则允许宿主为每个请求追加消息，这样的宿主会在每个 `model/call` 行里再次保存这些消息。
- Session 期间的 prompt 来源修订与工具采用：按构造，每次各自内联一次 `system` 或 `tools`。
- 请求事件上的 before 回调：产品装配没有注册（只有测试在 `model/call` 上注册）。草稿由验收测试覆盖（§18）。
- TypeScript 插件，以及用真实模型的 CLI 产品配置。

**假设。** 同一分支上的相邻请求在两次压缩之间互相延续。如果某个生产方每一步都改写一条早期消息，每行就要从第一条变化的消息起内联保存请求，每行的存储又会随请求增长：按消息字节增长，而 v16 是按每个引用 67 字节增长。目前产品中没有这样做的地方。那样做也会使 provider 的提示词缓存与 ContextPolicy 的基线失效，ContextPreparation 也已拒绝准备宿主这样做。§18 的验收测量每条事件的字节数，出现退化时能看出来。

## 5. 编码

### 5.1 base、keep 与根

- **请求事件**：`context/prepare` 或 `model/call` 事件；其 `data.request` 就是请求。before 轨迹中的草稿从不作为其他记录的 base。
- 分支 `b` 的**逻辑历史**：截至各自截点的祖先前缀，然后是 `b` 自己的事件。这就是运行时根据分支目录构建的祖先链 `[(branch_id, cut)]`，存储在 `timeline_branches.info` 中以 `parent` 与 `branch_point` 保存。
- `b` 上序号 `s` 的请求事件的 **base**：`b` 的逻辑历史中它之前的最后一个请求事件，即 `b` 上 `s` 之前的最后一个；没有时，取最近一个有请求的祖先上截点及之前的最后一个。
- **keep**：两个 `messages` 列表最长公共前缀的长度。每一对先用 `Arc::ptr_eq` 比较，再用 `Message` 的 `==` 比较。
- **根**：没有 base，或 `keep` 会是 0。根把所有消息、`system` 与 `tools` 都内联。
- **`system` 与 `tools`**：缺省 → `null`；存在且等于 base 的值（先指针再 `==`）→ `"base"`；否则内联。
- **草稿**：记录 before 轨迹中的请求（`WireTrace.original`、每个非空的 `WireStep.result`）以同样方式相对其所在记录的事件请求编码。该请求一定存在，因为草稿与其事件同一种类。

写入方总是取紧邻的上一个请求和最长前缀，所以字节是逻辑历史与该事件的函数。相等用类型自己的 `==`，这也是存储验收比较事件的方式（§18）。由此带来一点：消息内某个 JSON 对象值相等但序列化的键序不同时，算作保留，读回时采用其 base 的键序。相邻请求的消息由相同事件渲染，实际中不会出现，也没有读取方依赖键序。

为什么 `keep` 为 0 的请求是根：否则 `"base"` 的 system 或工具列表会沿链越过最近一次压缩，一直回到 Session 的第一个请求。有了这条规则，每条链都止于最近的根，最多回溯一次压缩（§7.4）。

### 5.2 持久化 JSON 类型

v16 的 `Row`、`WireTrace`、`WireStep` 与 `References` 不变。v17 为 `WireEvent` 与 `WireDraft` 各增加一个可选字段：

```text
WireEvent    = { event: AgentEvent, refs: References | null, delta?: RequestDelta,
                 trace: WireTrace | null, batch_end: integer }
WireDraft    = { event: EventDraft, refs: References | null, delta?: DraftDelta }
RequestDelta = { base: Position | null, keep: integer, messages: Message[],
                 tools: Inherit<ToolDefinition[]>, system: Inherit<SystemPrompt> }
DraftDelta   = { keep: integer, messages: Message[],
                 tools: Inherit<ToolDefinition[]>, system: Inherit<SystemPrompt> }
Position     = { branch?: string, sequence: integer }
Inherit<T>   = null | "base" | { inline: T }
```

- `delta` 只出现在 v17 的 `context/prepare` 或 `model/call` 记录及其草稿上，此时 `refs` 为 `null`。v16 请求记录有 `refs`、没有 `delta`。其他记录仍为 `refs: null`、没有 `delta`，字节与 v16 写出的完全相同。
- 与 v16 相同，存储的 `data.request` 为 `messages: []`，没有 `tools` 与 `system`；其余字段原样保留。
- `Position.sequence` 是 base 在 `Position.branch` 上的序号。base 在记录自己的分支上时省略 `branch`。位置不带 Session ID，所以分叉的逐字节复制仍然有效（§9）。
- `messages` 保存请求从下标 `keep` 起的消息，每条为 v16 `message` 内容正文的形式（`{role, content}`）。
- `base: null`（根）要求 `keep: 0`，且没有 `"base"`。
- `{inline: T}` 包一层，是因为 system prompt 本身可以是 JSON 字符串。
- `DraftDelta` 的 base 是其所在记录的事件请求，所以它没有 `base` 字段。

### 5.3 示例

某 Session 的三个请求行：`main` 上的一个根、`main` 上的下一个 prepare，以及在 `main` 10 处截出的子分支的第一个 prepare。为便于阅读显示为 JSON；该列实际保存字符串。

```json
[
  {"branch_id": "main", "sequence": 4, "kind": "context/prepare", "body": {"encoding": "event", "record": {
    "event": {"id": "e4", "sequence": 4, "create_at": 1004, "type": "context", "name": "prepare", "data": {
      "turn_id": "t1", "request_id": "r1", "through": 3,
      "request": {"providerId": "mock", "modelId": "deterministic", "messages": []}}},
    "refs": null,
    "delta": {"base": null, "keep": 0, "messages": [{"role": "user", "content": "Hi"}],
              "tools": null, "system": {"inline": "You are helpful."}},
    "trace": null, "batch_end": 4}}},
  {"branch_id": "main", "sequence": 9, "kind": "context/prepare", "body": {"encoding": "event", "record": {
    "event": {"id": "e9", "sequence": 9, "create_at": 1009, "type": "context", "name": "prepare", "data": {
      "turn_id": "t2", "request_id": "r2", "through": 8,
      "request": {"providerId": "mock", "modelId": "deterministic", "messages": []}}},
    "refs": null,
    "delta": {"base": {"sequence": 5}, "keep": 1,
              "messages": [{"role": "assistant", "content": "Echo: Hi"}, {"role": "user", "content": "Again"}],
              "tools": null, "system": "base"},
    "trace": null, "batch_end": 9}}},
  {"branch_id": "child", "sequence": 3, "kind": "context/prepare", "body": {"encoding": "event", "record": {
    "event": {"id": "c3", "sequence": 3, "create_at": 1013, "type": "context", "name": "prepare", "data": {
      "turn_id": "t3", "request_id": "r3", "through": 2,
      "request": {"providerId": "mock", "modelId": "deterministic", "messages": []}}},
    "refs": null,
    "delta": {"base": {"branch": "main", "sequence": 10}, "keep": 3,
              "messages": [{"role": "user", "content": "Follow up"}],
              "tools": null, "system": "base"},
    "trace": null, "batch_end": 3}}}
]
```

`main` 5 是 `r1` 的 `model/call`（`{"base": {"sequence": 4}, "keep": 1, "messages": [], "tools": null, "system": "base"}`），`main` 10 是 `r2` 的 `model/call`。各条链：

```text
 main                                                      child（父分支 main，截点 10）
  4 context/prepare  根    [u1] system 内联
  5 model/call       base 4，keep 1
  9 context/prepare  base 5，keep 1，[a1 u2]
 10 model/call       base 9，keep 3  <-----------------------  3 context/prepare  base main 10，keep 3，[u3]
 14 context/prepare  base 10，keep 3，[a2 u4]                  7 model/call       base 3，keep 4
```

## 6. 写入

### 6.1 找到 base

写入线程在写事务内编码每条记录。对 `(b, s)` 处的请求记录：

1. **记忆命中。** 如果记忆（§6.2）中有 `b` 的条目且其请求仍然存活，该条目就是 `b` 上最后一个请求行，其请求就是 base。
2. **否则**写入方先找位置：沿主键向后扫描自上次模型调用以来的行，取 `b` 上 `s` 之前的最后一个请求行；没有时，依据 `timeline_branches.info` 中的父分支与截点，依次在各祖先上取截点及之前的最后一个。然后在同一事务内用 §7.2 的链遍历解码该行；同一事务内此前写入的行也可见。
3. **找不到请求**：该记录是根。

随后写入方计算 `keep`、`system` 与 `tools`（§5.1），只序列化内联部分。各请求草稿按同样步骤相对记录的事件请求编码，该请求就在内存中。

### 6.2 写入方记忆

每个 `(Session, 分支)` 一条：写入方在该分支上编码的最后一个请求行的序号，以及指向其请求的 `Weak<ModelRequest>`。

- 记忆不持有请求：运行时丢掉请求后，条目即失效。每轮写入结束时移除失效条目，所以记忆只为“最后一个请求仍在内存中”的分支各保留一条小条目。
- 失败或回滚的事务移除其所涉 Session 的条目，`rollback_session` 与删除也一样。条目被移除只让下一次写入多做一次 §6.1 第 2 步的查找。
- 记忆不决定任何字节。命中时 base 是内存中的请求，未命中时从存储解码。比较按值进行，内联部分总是由正在写入的记录序列化，所以两者写出相同字节（§18 确定性）。它是存储内部的加速，不是业务状态。

测量结果（§4.2）：每种配置中每次追加请求时，上一个请求都仍然存活，所以运行中的 Session 写入时无需回读。重启、卸载或分叉之后，每个分支的第一次请求写入在写事务内付出一次链遍历，每个分支一次；其成本见 §7.4。

### 6.3 每个请求行的工作量

| | v16 | v17 |
| --- | --- | --- |
| 序列化 | 每条消息、tools 与 system | 只有 `messages[keep..]` 与变化了的 `system` 或 `tools` |
| 哈希 | 每份序列化内容的 SHA-256 | 无 |
| SQL | 每份内容一次 `INSERT OR IGNORE`，再写该行 | 写该行 |
| 比较 | 无 | 与 base 的前缀：`model/call` 行按指针，`context/prepare` 行按 `==`（§4.2） |
| 实测，SQL 之前，约 10k 条事件时最后 200 个请求行的中位数 | 普通 539 µs，工具 948 µs，产品窗口 495 µs | 普通 24 µs，工具 47 µs，产品窗口 39 µs |

v17 不写 `timeline_content` 行。输出行合并以及其他所有记录不变。

## 7. 读取

### 7.1 按序读取

每次读取保留它最后解码的请求及其位置：每次 `read_segments` 或 `read_page` 调用一份，`validate_session` 一轮中每个分支一份，像 #213 携带其缓存那样跨页携带。对每个请求行：

- **根**解码其内联部分；
- **base 是所保留位置的增量**，以所保留请求的前 `keep` 个消息 `Arc` 加上内联消息构成 `messages`，`system` 与 `tools` 取自所保留请求或内联值；
- **v16 行**经 `refs` 与 #213 的有界内容缓存解码；
- **base 为其他位置的增量**遍历链（§7.2）。

解码出的请求随后成为所保留的请求。草稿相对其所在记录的请求解码。沿逻辑历史进行的读取（`context`、`history`、完整的 `local` 与恢复读取）会发现每个 base 都已保留，只有运行时单独读取的某一段的第一个请求除外。

### 7.2 随机访问

base 未被保留时，用一次沿主键的递归查询从 base 回溯到最近的根、v16 行或所保留的请求，再从最早的一条起解码这条链：

```sql
WITH RECURSIVE chain(branch_id, sequence, delta, depth) AS (
  SELECT branch_id, sequence, json_extract(body, '$.record.delta'), 0
    FROM timeline_events WHERE session_id = ?1 AND branch_id = ?2 AND sequence = ?3
  UNION ALL
  SELECT e.branch_id, e.sequence, json_extract(e.body, '$.record.delta'), c.depth + 1
    FROM chain c JOIN timeline_events e
      ON e.session_id = ?1
     AND e.branch_id = coalesce(json_extract(c.delta, '$.base.branch'), c.branch_id)
     AND e.sequence = json_extract(c.delta, '$.base.sequence')
   WHERE c.depth < ?4 AND NOT (c.branch_id = ?5 AND c.sequence = ?6))
SELECT branch_id, sequence, delta FROM chain ORDER BY depth DESC
```

`?4` 是该 Session 的行数，损坏的链因此不会无限循环；`?5`、`?6` 让遍历停在所保留的请求处。没有增量的行结束这条链：它是 v16 行，经其引用解码。`EXPLAIN QUERY PLAN` 显示每一步都是主键查找。每一步都在 Rust 中校验（§8）。

随机访问只在不从逻辑历史起点开始的读取调用中发生，每次调用一次：从历史中间开始的页、按事件 ID 读取、`trace`、快照锚点、从 `resume` 起的检查点后缀读取、分支自身的段（与其祖先分开读取）、校验轮中子分支的第一个请求，以及写入方记忆未命中。它不按行发生。

### 7.3 读取路径

```text
 一次读取调用（read_segments、read_page、一轮校验、按 ID 读取、trace）
   kept = 无
   按序号依次处理每一行：
     v16 请求行   --refs--> 内容行（#213 缓存）-------------------------------> 请求 --+
     v17 根       --内联部分------------------------------------------------> 请求 --+
     v17 增量，base = kept 的位置                                                     |
                  --kept.messages[..keep] + 内联；system/tools 为 "base" 或内联 --> 请求 --+
     v17 增量，其他 base                                                              |
                  --一次递归主键查询：base -> ... -> 根 | v16 行 | kept                |
                    从最早的一条起解码，再解码本行 ------------------------------> 请求 --+
                                                                         kept = 请求 <--+
     本行的草稿：所在记录的请求 + DraftDelta
```

### 7.4 链长，以及为什么不设上限

一条链从某个请求回溯到它之前最近的根：最多是最近一次压缩或 Session 开始以来的请求行。正常情况下（重试除外），每个请求行至少增加一条消息（一轮的输入、模型输出、一个工具结果），所以链的行数与被解码请求的消息数大致相同。v16 随机访问同一个请求时，每条消息做一次内容查找和 SHA-256 校验，所以两者都与请求大小成线性关系；v17 用主键步进取代了内容查找。

在探针的数据库上测量（§12），取五次的中位数；请求是该 Session 最后一个 `model/call`，v16 一列是 `main` 上存储自己读取这一条事件：

| 配置 | 事件数 | 回溯到根的链行数 | v16 读取该事件 | v17 链查询与解码（原型） |
| --- | --- | --- | --- | --- |
| 普通 | 1,001 | 200 | 0.48 ms | 0.50 ms |
| 普通 | 3,001 | 600 | 2.7 ms | 1.7 ms |
| 普通 | 10,001 | 2,000 | 9.2 ms | 8.8 ms |
| 工具 | 10,009 | 2,224 | 11.1 ms | 11.3 ms |
| 产品窗口 | 3,007 | 27 | 0.48 ms | 0.23 ms |
| 产品窗口 | 10,017 | 41 | 0.69 ms | 0.32 ms |

写入方记忆未命中（§6.2）走的是同一遍历：距上一个根 2,000 个请求行时，在写事务内约 9 ms，重启后每个分支一次；有压缩时远低于 1 ms。

设链深上限就是每 N 个请求强制一个根：每次重新保存整个请求（最大到窗口），为了省下一项本来就与 v16 解码同一请求相当、且每次读取调用只付一次的成本，把存储随历史增长的问题又带回来。因此不设上限。

还剩一项成本与两次压缩之间调用次数的平方相关：事件类型要求每个解码出的请求带上全部消息指针（`messages: Vec<Arc<Message>>`）。10k 条普通事件的一次全历史读取要构建 2,000,000 个指针，约占原型 29.6 ms 中的 7.7 ms（§12）。v16 构建同样的向量；v17 去掉的是其背后的查找与解析。第 3 阶段（§15）去掉的是恢复期间整次读取的重复。

## 8. 校验与损坏行

对每条 v17 请求记录，读取检查以下各项，不满足时以存储错误失败：

- 请求事件恰好有 `refs`（v16）或 `delta`（v17）之一，且有 `delta` 时 `refs` 为 `null`；其他事件两者皆无；草稿与所在记录采用同一编码；
- `base: null` 时：`keep` 为 0，且 `system`、`tools` 都不是 `"base"`；
- 有 base 时：该行存在；它是种类为 `context/prepare` 或 `model/call` 的单事件行；它在逻辑历史中更早，即同一分支上序号更小，或位于本分支祖先链上的某个分支、且在该祖先的截点及之前（依据 `timeline_branches.info` 检查）；
- `keep` 不超过 base 的消息数；`system` 或 `tools` 为 `"base"` 时，base 必须有对应的值；
- 增量能按上述类型解析，解码出的请求能按 `ModelRequest` 解析。

base 缺失或损坏时，读取明确失败（“missing request base”“invalid request base”）；请求绝不会被还原为空或残缺。坏行保留原字节：读取从不写入。`validate_session` 对每一行做同样检查，所以链断开的 Session 在恢复时被拒绝并保留，与今天缺少内容行时一样。一条损坏行的影响范围与 v16 相同：此后依赖它的每个请求，直到下一个根（v16 中是引用了损坏消息的每个请求）。

## 9. 分支、分叉与删除

- **跨分支 base。** 子分支的第一个请求以最近一个有请求的祖先上截点及之前的最后一个请求为 base，用 `branch` 寻址。子分支后续请求以子分支自己的行为 base。父分支在截点之后继续写入不影响子分支：截点之后的行不在子分支的逻辑历史中。
- **分叉**（[S5c §6.3](session-deletion-and-fork.md#63-分叉复制什么)）。截至截点的 `main` 行逐字节复制。`main` 没有祖先，所以每个被复制行的 base 都是更早的 `main` 行，同样被复制。复制的子分支目录头部为 0，没有事件。v16 行仍需要其内容行：分叉照旧复制被复制行引用的内容行，v17 行不引用任何内容行。分叉事务不变。
- **删除与创建回滚**照旧从每张表删除该 Session 的行；写入方丢弃该 Session 的记忆条目。

## 10. 压缩

完成的压缩改变列表开头，所以下一个请求与其 base 没有公共前缀。它是根，把 replacement、基线、保留的尾部、`system` 与 `tools` 内联一次，大小受模型窗口限制。之后的请求延续它。在产品窗口配置中，每次完成的压缩恰好产生一个根（§4.2）。

```text
 序号  事件                       messages                         存储
  37  model/call                 [u1 a1 ... u19]                  base 36，keep 37
  40  context/prepare            [u1 a1 ... u19 a19 u20]          base 37，keep 37，[a19 u20]
  41  context/compaction-start
  42  context/compaction         completed：replacement S，从 u18 起原样保留
  43  model/call                 [S b1 u18 a18 u19 a19 u20]       根：7 条全部内联一次，最多一个窗口
  50  context/prepare            [S b1 u18 ... u20 a20 u21]       base 43，keep 7，[a20 u21]
```

检查点读取从快照的 `resume` 开始，它位于压缩之前。其中第一个请求行沿链回溯到上一个根，最多是两次压缩之间的请求行；压缩之后的行链接到新的根。

## 11. 格式 v17

**决定：版本 17，从恰为 v16 的库原地改标记。表、列、索引都不变，不读取、不改写任何业务行。** 以提议形式记录在 [native-storage-format.md](../../../design/appserver/native-storage-format.md#格式版本)；完整的格式决定随实现移到那里。

- **为什么要新版本。** v16 守护进程读不了 v17 请求行：`WireEvent` 与 `WireDraft` 拒绝未知字段，而没有 `delta` 请求就是空的。格式只在旧读取器能正确解释新数据时保持版本。
- **结构。** v16 的表原样保留；`agent_store_format` 改为 `CHECK (version = 17)`。`timeline_content` 保留：它保存 v16 行的内容，v17 仍要读取。新的 v17 Session 不往里写。
- **混存。** v16 行保持写入时的样子，永久可读。升级后，Session 的下一个请求行是 v17，以其 v16 前驱为 base（`keep` 通常为其全长）。两种版本的行可在同一分支、同一数据库中混存，分叉原样复制它们。

| 发现的库 | 启动时的处理，在现有准入事务内（`IMMEDIATE`，写入型 PRAGMA 之前） |
| --- | --- |
| 空库 | 建 v17 结构，标记 17 |
| 恰为 v17 结构，标记 17 | 打开 |
| 恰为 v16 结构，标记 16 | 重建 `agent_store_format` 为 `CHECK (version = 17)` 并写入 17，别的都不做；失败则回滚，原 v16 库保持不变 |
| 恰为 v15 或 v13 结构且标记相符 | 先走已有各步，再走本步，在同一事务内 |
| 其他 | 照旧拒绝并保留 |

升级是单向的，与 S5a、S5b 的决定相同：v16 守护进程以现有的“unsupported SQLite schema … preserved”错误拒绝升级后的库。升级提交后，守护进程向 stderr 写一次 `Upgraded native store <path> from format v16 to v17; earlier dimcode versions cannot open it.`。在 `sqlite.rs` 中，这是 `UPGRADES` 里的一项 `(16, &[])`，加上 `FORMAT = 17`。

不回填：转换 v16 行要读取并改写业务行，升级不得这样做，而 #213 已经限定了读取它们的成本。

## 12. 预估效果

预计存储量，依据探针为每个请求行构建的精确 v17 增量 JSON；其他行不变。字节数为事件行的 `body` 文本，v16 另加内容行的 `kind`、`hash` 与 `body`。

| 配置 | 事件数 | v16 请求行 | v17 请求行 | v16 每条事件 | v17 每条事件 | v16 合计 | v17 合计 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 普通 | 1,001 | 7,248 B | 560 B | 1,720 B | 352 B | 1.72 MB | 0.35 MB |
| 普通 | 3,001 | 20,650 B | 548 B | 4,398 B | 351 B | 13.2 MB | 1.05 MB |
| 普通 | 10,001 | 67,551 B | 545 B | 13,778 B | 351 B | 137.8 MB | 3.51 MB |
| 工具 | 1,009 | 8,052 B | 605 B | 2,097 B | 400 B | 2.12 MB | 0.40 MB |
| 工具 | 3,007 | 22,928 B | 595 B | 5,401 B | 399 B | 16.2 MB | 1.20 MB |
| 工具 | 10,009 | 75,055 B | 592 B | 16,985 B | 399 B | 170.0 MB | 3.99 MB |
| 产品窗口 | 3,007 | 4,575 B | 2,473 B | 1,613 B | 983 B | 4.85 MB | 2.95 MB |
| 产品窗口 | 10,017 | 4,707 B | 2,506 B | 1,638 B | 989 B | 16.4 MB | 9.91 MB |

请求行一列是所有请求行的平均值；不计 SQLite 页、键与索引。没有压缩时，v16 每条事件的字节数随历史增长，v17 保持不变。约每 20 轮压缩一次、消息 1 KB 时，v16 本已有界，v17 仍少存 40%：v16 的每行为请求的每条消息重复一个 67 字节的引用，v17 的根每次压缩重复一次保留的尾部。

读取。探针还把每个 Session 复制到第二个一次性数据库，把每个请求行改写为其 v17 形式，并用一个 v17 原型读取器解码（解析行与事件，把增量应用到所保留的请求上）；它返回的事件与 v16 存储读取同一 Session 的结果相等。同一机器、同一次运行；v16 为 `main` 上的存储，已含 #213。其 10k 条事件的读取（普通 149 ms，工具 180 ms）与 #213 自己的测量（146 与 184 ms）一致。

`main` 的全历史读取，五次的中位数：

| 配置 | 事件数 | v16 存储读取 | v17 原型读取 | 构建的消息指针 |
| --- | --- | --- | --- | --- |
| 普通 | 1,001 | 3.8 ms | 2.3 ms | 20,000 |
| 普通 | 3,001 | 20.5 ms | 7.0 ms | 180,000 |
| 普通 | 10,001 | 149.0 ms | 29.6 ms | 2,000,000 |
| 工具 | 1,009 | 4.4 ms | 2.5 ms | 25,088 |
| 工具 | 3,007 | 24.3 ms | 7.9 ms | 223,112 |
| 工具 | 10,009 | 179.6 ms | 32.8 ms | 2,473,088 |
| 产品窗口 | 3,007 | 15.8 ms | 9.6 ms | 31,759 |
| 产品窗口 | 10,017 | 59.8 ms | 37.0 ms | 109,576 |

运行期间机器上还有其他工作；每次重复中 v16 与 v17 交替进行，所以对比是公平的，但绝对耗时带有这部分噪声。

**预估（不是在实现上测得的）。** 把原型的普通与工具读取拟合为 `a'·E + c·P`（`P` 为消息指针数），得到每个指针 c ≈ 3.5–3.9 ns，每条事件 a' ≈ 2.2–2.4 µs。实现会像现在的存储一样校验每个增量、处理分支与输出行，所以预估采用 §2.3 中存储自己的每事件成本（a ≈ 2.9 µs），再加上指针项。

| 约 10,140 条事件时 | #213 之前 | 有 #213 | v17，预估 |
| --- | --- | --- | --- |
| 全历史读取，普通 | 208 ms | 146 ms | 30–37 ms |
| 恢复加首次 idle，普通 | 2,866 ms | 1,779 ms | 0.38–0.47 s |
| 全历史读取，工具 | 257 ms | 184 ms | 33–39 ms |
| 恢复加首次 idle，工具 | 3,624 ms | 2,256 ms | 0.44–0.51 s |
| 请求行编码（SQL 之前），普通 | 539 µs | 539 µs | 24 µs |

- 一次全量读取约为 `2.9 µs × 10,140 + c × P`；普通轮构建 P ≈ 2.06 M 个指针，工具轮约 2.5 M。下限取原型在 10,001 与 10,009 条事件时实测的读取。
- 一次恢复做 11 次全量读取，外加约等于一次读取的 `validate_session`（§2.1、§2.2）；#213 之后其余工作约 24 ms（普通）与 46 ms（工具）。所以恢复约为 12 次读取加上这部分。
- 从存储读取上下文历史的模型步骤，付出的是每次读取的那个数字。
- 剩下的二次项是指针向量：10k 条普通事件时每次读取约 8 ms；如果某个 Session 到 30k 条事件仍不压缩，约 70 ms。第 3 阶段（§15）让每次恢复只付一次，而不是十二次。

验收 7 在实现上测量这些数字。

## 13. 影响面

按[设计规则 §3](../../../architecture/rust-agent/principles.md#3-正交)：目标是存储之外零改动、零新增公开内容。

| 模块或内容 | 改动 | 理由 |
| --- | --- | --- |
| `dim-app-server` 存储（`codec.rs`、`timeline.rs`、`sqlite.rs`） | 编码、解码、写入方记忆、链查询、`FORMAT = 17` 与一个升级步骤 | 格式是存储自己的事（timeline-storage §2） |
| `TimelineStore` trait、Timeline、Dispatcher、事件类型、样本、能力、协议 | 无 | §3 |
| 新增公开内容 | 无 | 持久化 JSON 是存储内部的，由其标记管理版本；没有事件、服务、方法、wire 字段或配置 |

## 14. 规则检查

| 规则 | 回答 |
| --- | --- |
| [设计规则 §4](../../../architecture/rust-agent/principles.md#4-通过时间线扩展行为) 第 1 条（改别人的状态写指令事件） | 不适用：不改变其他模块的状态 |
| 第 2 条（交接） | 不适用：没有交接 |
| 第 3 条（before 回调） | 不新增、不修改。回调照旧收到并返回完整草稿；存储写入记录时，在 before 链运行之后，把轨迹中的请求草稿相对所在记录的请求编码（§5.1） |
| 第 4 条（核心机制不带业务含义） | 存储按结构（位置、前缀长度）编码 `ModelRequest`，不解释任何消息或业务规则。哪些事件带请求，v16 的存储本来就知道 |
| 第 5 条（原子批次） | 不变。base 总在同一事务或更早的事务中提交，因为批次整体提交且 base 位于该行之前；订阅者看到的事件不变 |
| 第 6 条（模型上下文） | 不变：读取返回相同事件，上下文因此相同 |
| 第 7 条（公开事件与服务） | 存储不读取任何插件的状态；写入方记忆属于存储自己 |
| 第 8 条（基础设施默认不变） | Timeline、Dispatcher、事件信封与格式、装配和门面都不变。需求来自存储自身（其读写成本），改动与验证都在存储内闭环 |
| 第 9 条（只有后续决定要用的事实进 Timeline） | 不新增事件或事实；编码不是 Timeline 事实 |
| §1（Capability 与 App Server 边界） | 在现有 SessionStore 插件的 `TimelineStore` 实现内；不新增插件 |
| [ADR 002](../../adr/002-plugin-scope.md) D1（作用域随状态） | 进程作用域：存储及其写入线程 |
| ADR 002 D2（不设接线模块） | 不新增模块或插件 |
| ADR 002 D3（依赖随作用域） | 不新增依赖 |
| ADR 002 D4（配置） | 无 |
| ADR 002 D5（语言中立合同） | 不改合同；§5.2 以数据形式定义存储的 JSON |

## 15. 范围之外，以及向机制所有者提出的需求

不提出需求。以下作为单独工作，不在本设计范围内：

- **第 3 阶段：恢复范围内的共享读取。** Session 恢复期间，每个能力都对同一个 `S` 读取 `local(..=S)`。由恢复共用一次读取、只解码一次、在 Session 发布时释放，可以在不改任何插件的情况下去掉剩下的倍数。这由运行时所有者决定，将在 v17 合入后附带数据提出。
- 转换已有的 v16 行（§11）。
- 任何事件类型的改动（§17）。

## 16. 决定

| # | 决定 | 否决的方案 | 理由 |
| --- | --- | --- | --- |
| D1 | base 总是逻辑历史中紧邻的上一个请求 | 前缀最长的更早请求；写入方手里还有的任意请求 | 一条确定的规则；搜索会让字节依赖写入方的状态，且每次写入要扫描 |
| D2 | `keep` 为 0 即为根 | 允许没有公共消息的 base | 每条链止于最近的根，`"base"` 的 system 或工具列表不会沿链越过压缩（§5.1） |
| D3 | base 以 Session 内的 `{branch?, sequence}` 寻址 | 事件 ID；带 Session 的地址 | 每跳一次主键查找，不需要索引；位置在分叉的逐字节复制后仍然有效（§9） |
| D4 | 草稿相对所在记录的请求编码，没有 `base` 字段 | 草稿自成一条链；草稿用 v16 引用 | 所在记录的请求在两端都在内存中，草稿也从不作为 base |
| D5 | `system` 与 `tools` 为 `null`、`"base"` 或 `{inline}` | 各自引用保存该值的行 | 有了 D2，链本来就会到达保存它们的行；不需要另一种地址 |
| D6 | 写入方记忆每分支保存一个位置与一个 `Weak`，事务失败时丢弃、失效时清除 | 强引用记忆；不设记忆 | 强引用会持有进程写过的每个 Session 的最后一个请求；没有记忆则每次写入都要回读 base（§6.2） |
| D7 | 不设链深上限 | 每 N 个请求强制一个根 | §7.4 |
| D8 | 不改 SQL；链查询在主键上使用 `json_extract` | `base` 列或索引 | 升级只增加结构，而链只需要主键 |
| D9 | v16 行保持写入时的样子 | 升级时或惰性转换 | 升级不读取或改写任何业务行（§11） |
| D10 | v17 请求记录保留 `refs: null` | v17 省略 `refs` | 只有一种 `WireEvent` 形状；所有非请求记录保持其 v16 字节 |

## 17. 否决的方案

- **内容寻址的列表节点**（每条消息一个节点，指向前一节点的哈希）：每次写入仍要哈希，前缀变化的请求要改写 O(k) 个节点。
- **推导请求而不保存**：`model/call` 是交给模型的最终请求（trace.md），ContextPolicy 读取精确的最近请求；推导是同一事实的第二个生产方。
- **事件中的惰性请求句柄**：改变事件类型和所有使用方，并把存储错误带进期待已校验数据的读取方（AGENTS.md 数据约定）。
- **按种类过滤的读取**（为不关心请求事件的能力跳过它们）：改变读取 API 与各能力的重建，且二次方的存储依然存在。
- **能力快照**用于重建：在每个插件里为优化增加业务状态（timeline-storage §4）。
- **存储范围的已解码事件缓存**：驻留会随每个已装载或最近读取的 Session 增长，违反空闲分支验收。
- **把 `ModelRequest.messages` 改为持久化数据结构**：为了指针复制这一项（§7.4）改变事件类型及其冻结合同。

## 18. 验收

实现 PR 须全部通过，且只使用一次性数据库。

沿用自 [timeline-storage §7](../../../architecture/rust-agent/timeline-storage.md#7-验收)：

1. **存储还原。** 读回的事件与写入的逐字段一致，覆盖跨写入轮追加的输出行、原子批次与 before 轨迹，另外覆盖：根，`keep` 为 0（压缩）、部分与全长；`system` 与 `tools` 为 `null`、`"base"` 与内联；请求草稿不变、追加一条消息、改动早期消息、改动 system prompt；以父分支和祖父分支为 base 的跨分支 base，且父分支在截点后继续写入；混存行（升级后继续的 v16 Session，其第一个 v17 行以 v16 行为 base）；以及在混存 `main` 的截点处分叉，分叉的读取等于源的前缀。
2. **长会话驻留。** 2,000 步、800 KB 上下文：空闲分支的存活字节不随步数增长，准备期间的峰值不随压缩前的历史增长。Session 卸载后，写入方记忆不再持有它的任何存活请求。
3. **分页与恢复。** 长历史页读不先解码全量；从历史中间开始的页，以及从快照 `resume` 起的检查点读取，各用一次链查询解码其第一个请求；合并行、批次与跨页 ToolCall 仍然正确。恢复缓存有界（每次读取保留一个请求，v16 行用 #213 的缓存），长冷读取不占用连接以致阻塞写入。
4. **进程中断与断电。** 在提交前、提交后发布前、发布后终止：恢复的事件能读回，序号连续，没有行的 base 未提交。重启后（记忆为空）写出的字节与未中断的运行相同。
5. **存储故障。** 注入读取失败与提交失败时行为照旧；提交失败后，重试与之后的批次只相对已提交的行编码，字节与未发生故障的运行相同。
6. **损坏。** base 缺失、base 更晚、位于祖先链之外的分支、越过截点、种类不符或是输出行、`keep` 超过 base 的长度、没有 base 值却为 `"base"`、增量格式错误、成环：每种都让 `read`、`read_page` 与 `branches` 以存储错误失败；该行保留原字节，没有请求被读回为空。#213 的内容损坏用例仍然失败。

新增：

7. **两次压缩之间的大量模型调用。** 普通与工具 Session 不压缩地增长到约 1k、3k、10k 条事件（`restore_cost`）。报告每个规模下的恢复加首次 idle、每次读取耗时与每条事件的字节数，并与 #213 的数据对照。10k 时每条事件的字节数与 1k 相差不超过 10%，普通轮 10k 条事件时，每次读取耗时不超过 #213 的三分之一，恢复加首次 idle 不超过其一半（此处所用机器上为 1,779 ms），在同一台机器上测量。
8. **确定性。** 同一段历史（含子分支、一次压缩与请求草稿）分别在启用写入方记忆、禁用记忆（`testing` 钩子）、中途重启的情况下写入，`timeline_events` 各行逐字节相同。
9. **冻结合同。** `crates/dim-agent-types/tests/fixtures/` 与 `*_contract.rs` 测试不被改动，且通过。
10. **升级。** 空库建为 v17。一次性 v16 库原地升级为精确的 v17 结构，所有行逐字节不变，并只报告一次；v15 与 v13 库在一个事务内到达 v17；标记不符、升级到一半、更旧与未知的结构被拒绝并保留；v16 守护进程拒绝 v17 库时使用现有错误。

## 19. 对其他文档的改动

随本设计：[文档索引](../../../INDEX.md)链接本页，[native-storage-format.md](../../../design/appserver/native-storage-format.md#格式版本) 把 v17 列为提议的下一版本。

随实现：[native-storage-format.md](../../../design/appserver/native-storage-format.md)（当前版本、标记、`timeline_content` 改为只保存 v16 内容、持久化 JSON 类型、读取校验、v17 决定与完整记录示例），[timeline-storage](../../../architecture/rust-agent/timeline-storage.md) §2（请求内容与读取）、§4（恢复行）与 §7（验收行），[ADR 001](../../../adr/001-harness-foundation.md)（当前格式），以及本页状态。

## 20. 待评审的问题

1. **`keep` 按值相等。** 写入方用 `Message` 的 `==` 比较消息；在这种相等下，键相同但键序不同的 JSON 对象相等，所以这样的消息读回时采用其 base 的键序（§5.1）。逐字节比较要为每个 `context/prepare` 行序列化其保留的前缀，而这些消息从不与 base 共享指针（§4.2），这正是 v17 去掉的写入成本。建议：用 `==`。有没有读取方依赖消息内容中的键序？
2. **校验对 base 要求多严。** §8 接受逻辑历史中任意更早的请求事件作为 base；“总是紧邻的上一个请求”（D1）是写入方规则，由验收 8 测试。读取时检查 D1 能在恢复时发现写入方的缺陷，但会拒绝一个能正确解码的库，而且以后对写入规则的任何改动都会变成格式变化。建议：校验解码所需的条件，测试 D1。
3. **第 3 阶段由谁负责。** 恢复范围内的共享读取（§15）属于运行时。由谁负责？是否等 v17 的实测数据？
4. **不备份的单向升级。** S5a 与 S5b 在开发阶段这样决定，v17 沿用。对发布 v17 的版本是否仍然成立，还是升级应保留一份 v16 数据库副本？
