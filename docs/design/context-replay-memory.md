# 上下文重建的内存计划

Status: Draft。交付状态在 issue，不在本文。
Issue: #708。

Scope: branch。
Rules: [design rules](../architecture/rust-agent/principles.md)、[ADR 002](../adr/002-plugin-scope.md)。

## 1. 范围与非目标

范围：一次模型步骤里 `Timeline` → `HistoryRenderer` → `ContextPreparation` → `ContextPolicy` →
`ModelRequest` 这条重建链的分配形状，以及重放的边界。

非目标：

- 不减少模型步骤数，不改变压缩、快照、中断或配置采用语义。
- 不改变 Timeline 事件、存储格式、wire protocol 或 `ModelRequest` 的序列化。
- 不把历史重新做成常驻缓存。
- 不先做分配器调参；它只作为最后兜底（§8）。

## 2. 问题与证据

环境：`dimcode-server` 0.1.0-alpha.11（release manifest 的 sourceCommit `35b3a06`），产品 home，
Linux x86_64，glibc 2.39，运行约 1.5 小时。只读观察，未改产品数据。

| 指标 | 值 |
| --- | --- |
| RSS / 匿名 PSS | 1.62 GiB / 1.62 GiB |
| malloc system / free / in-use | 1726 MiB / 1619 MiB / **107 MiB** |
| `malloc_trim(0)` 后 RSS | **428 MiB**（释放约 1.20 GiB） |
| arena | 38 个；两个 764.5 / 488.2 MiB（峰值 809 / 1350 MiB） |
| 空闲块 ≥512 KiB | 273 个，合计约 872 MiB |

数据库：106 个 Session、13,831 个事件、969 次 `context/prepare`、963 次 `model/call`，
平均事件正文 1,419 B。

最大会话：

| 指标 | 值 |
| --- | --- |
| 事件数 | 4,903 |
| prepare 次数 | 436 |
| 累计重放事件（prepare 的 sequence 之和） | 1,086,523 |
| 最新输入 | 510,957 tokens / 1,000,000 window |
| `context/compaction` | 0 |
| `timeline_snapshots` | 0 |

最大会话的累计重放约 1.5 GB，与它所在 arena 的 1.35 GiB 峰值同一量级；这支持“该 arena
保留的空闲块来自这个会话的重放缓冲”。全库累计 2.2 GB 与 1.62 GiB RSS 相比，不能单独证明
所有 Session 都没有复用；跨 Session 的复用结论是 P0 要验证的假设，不是本文已证明的事实。

结论：

1. 重建结果确实被释放了：in-use 只有 107 MiB，`malloc_trim` 当场归还 1.2 GiB。
2. 最大会话的 arena 高水位与它的累计重放量同量级，说明这些缓冲在它的线程上没有复用；
   是否适用于所有 Session 由 P0 测量。
3. glibc 把这些空闲块留在 arena 里；动态 mmap 阈值让释放过的大块后续也走堆。
   这是保留现象，不是根因。

## 3. 现状：一次 prepare 实际造了几份

```text
Timeline::context_from
  └─ events: Vec<Arc<AgentEvent>>          整段历史物化              (1)
       └─ HistoryEntry { message: Message } 文本复制                  (2)
            ├─ context_messages()           整份克隆：前缀校验         (3)
            ├─ context_messages()           整份克隆：取长度           (4)
            ├─ context_messages()           整份克隆：组请求           (5)
            └─ ModelSettings::request()     每条消息再 Arc::new        (6)
```

对应代码：

| 位置 | 行为 |
| --- | --- |
| [timeline.rs:805](../../crates/dim-agent-runtime/src/core/timeline.rs#L805) `context_from` | 从 sequence 1 的整段历史装进 `ContextHistory.events` 和 `positions` |
| [history.rs:10](../../crates/dim-agent-capabilities/src/history.rs#L10) `HistoryEntry` | `message: Message`，文本由事件复制而来 |
| [history.rs:555](../../crates/dim-agent-capabilities/src/history.rs#L555) `context_messages` | 对每条消息 `entry.message.clone()`；有压缩时还克隆 `replacement` 和 `context_baseline[].message`（两者仍是 `Message`） |
| [context_preparation.rs:542](../../crates/dim-agent-capabilities/src/context_preparation.rs#L542) | 为校验前缀整份克隆 |
| [context_policy.rs:271](../../crates/dim-agent-capabilities/src/context_policy.rs#L271) | `context_messages(...).len()`，只为取长度整份克隆 |
| [agent_loop.rs:671](../../crates/dim-agent-capabilities/src/agent_loop.rs#L671) | prepare 路径组请求时的第三次整份克隆 |
| [host.rs:99](../../crates/dim-agent-capabilities/src/host.rs#L99) `ModelSettings::request` | `messages.into_iter().map(Arc::new)`，每条消息一次分配 |

`configured_request`（[context_policy.rs:221](../../crates/dim-agent-capabilities/src/context_policy.rs#L221)）也调用 `context_messages`，
但它出现在 usage（[context_preparation.rs:481](../../crates/dim-agent-capabilities/src/context_preparation.rs#L481)）和
compaction（[agent_loop.rs:359](../../crates/dim-agent-capabilities/src/agent_loop.rs#L359)）路径，不在一次 prepare 的三次克隆里。

每一步的缓冲都比上一步大（历史在增长），旧洞装不下新块；事件、投影、消息又交错分配，
释放后洞不连续。于是 `free` 了，但下一次用不上。

压缩边界本可以避免全量重放，但快照只在压缩完成后写入。这个会话 51 万 / 100 万 tokens，
尚未压缩，快照为 0，所以每一步都从 sequence 1 重放。

## 4. 目标

- 为取长度、校验前缀、组请求不再物化整份上下文。
- 一次 prepare 的分配量只跟**新增事件**和**新增消息**有关，不跟历史长度成正比。
- 冷重建的峰值是一段，而不是整段历史。
- 重建次数、压缩与快照语义不变；重载后仍能从 Timeline 重建同样的投影。

## 5. 方案

### P0 测量基线（先做）

新增 opt-in example（不进 CI）：mock model 造 400–500k token 上下文、100+ 步、无压缩，
每步记录 Rust live/peak（现有 `MeasuredAllocator`）与 Linux RSS，输出 JSON。
改动前先跑一遍，作为 P1–P3 的对照。

### P1 共享消息，去掉深拷贝（#709）

- `HistoryEntry.message` 改为 `Arc<Message>`。
- `context_messages` 返回 `Vec<Arc<Message>>`，只克隆指针。
- `ModelSettings::request` 接收 `Vec<Arc<Message>>`，去掉 `map(Arc::new)`。
- `ModelRequest` 的序列化不变（它本来就是 `Vec<Arc<Message>>`）。
- 压缩的 `replacement: Vec<Message>` 和 `ContextBaselineEntry.message: Message` 保持原类型：它们是事件数据，
  改动会牵动冻结 fixture，大小也受压缩结果限制。有压缩的会话这两项仍按 `Message` 克隆；P0 若显示它们显著，再单独评估。

净分配：当前 `ModelSettings::request` 已经对每个请求的每条消息做一次 `Arc::new`；把 `Arc` 放进
`HistoryEntry` 是用克隆替换这些分配，不是新增。P1 净减少的是 `context_messages` 的整文本克隆和
每条消息的 `Arc::new`。

收益：每次 prepare 少 2–3 份整文本拷贝；前缀校验与取长度可以直接读条目。
事件 JSON 不变，冻结 fixture 不动。

### P2 去掉元数据物化

- 前缀长度由 `replacement.len() + context_baseline.len() + history.len()` 算出，不再造 Vec。
- 前缀校验按引用迭代比较 `&Message` 与 `&Arc<Message>`，不克隆。
- usage/compaction 的 `configured_request` 也接收同一份 `Arc<Message>` 列表，不另造；它不在 prepare 路径上。

### P3 按边界重放（核心，#710）

每次 prepare 保留渲染器的投影状态，下一次只处理新增事件。保留值属于 branch：

```rust
struct ProjectionResume {
    position: TimelinePosition,        // 已消费到的 (branch, sequence)
    state: ProjectionCheckpointState,  // retain_from, prompts, delivered, inputs, configuration, output, open_tools, output_open
    entries: Vec<HistoryEntry>,        // 前缀；P1 后 message 是 Arc<Message>
    origins: Vec<usize>,               // entry 到事件下标的映射
}
```

- **读取**：同一分支上用 `Timeline::local(position.sequence + 1..=through)`
  （[timeline.rs:718](../../crates/dim-agent-runtime/src/core/timeline.rs#L718)）。事件按序交给
  `HistoryRenderer` 的续接函数；现有 `replay(events, initial)` 已接受
  `Option<&ProjectionCheckpointState>`（[history.rs:291](../../crates/dim-agent-capabilities/src/history.rs#L291)），
  新增 crate 内续接 API 返回新的 entries/origins/state。
- **不复用 `ProjectionCheckpoint`**：`decode_checkpoint` 只接受锚定 completed `context/compaction`
  的快照（[history.rs:77](../../crates/dim-agent-capabilities/src/history.rs#L77)），而这里的 resume 是
  上一次 prepare，可能没有压缩。持久快照和 `context_with_checkpoint` 保持原样。
- **提交点**：把现有 `prepared: Mutex<Option<(String, u64, Arc<ModelRequest>)>>`
  （[context_preparation.rs:295](../../crates/dim-agent-capabilities/src/context_preparation.rs#L295)）
  换成 `PreparedStep { turn_id, through, request, resume }`。当前写入点
  [context_preparation.rs:534](../../crates/dim-agent-capabilities/src/context_preparation.rs#L534) 在渲染和前缀校验之前；
  改为在前缀校验成功后与 request 一起写一次。取消、host prepare 失败或前缀校验失败都不推进 resume；
  下一次仍从上一个成功边界继续。
- **失效与回退**：没有 resume（首次/重载）、resume 不在当前分支、`through < position.sequence`、
  position 之后出现 `context/compaction`、续接返回错误，或与全量重建的前缀不一致时，用
  `rendered_context` 全量重建并替换。prompt/config/input/tool 事件在 position 之后按序应用；
  它们已经在 `ProjectionCheckpointState` 里（`inputs`、`delivered`、`prompts`、`configuration`、
  `open_tools`、`output`、`output_open`，[checkpoint.rs:40](../../crates/dim-agent-types/src/checkpoint.rs#L40)）。
- **不保留原始历史**：保留的是前缀 entries 和状态，也就是模型请求本来就需要的内容；重载时全量重建一次。
- **与快照的关系**：持久快照仍是跨进程/跨装载的加速；内存 resume 只服务同一装载内的连续 prepare。

这一步把每步分配从 O(历史) 降到 O(新增事件)，是关键修复。

### P4 冷重建分段（可选，暂不实施）

全量重建（首次、重载、边界失效）目前仍用 `rendered_context` 一次物化。若 P3 之后冷重建峰值仍然明显，
再向 runtime 提出分段读取要求；本计划不实施，也不向 runtime 提要求。

## 6. 验收与测量

| 项 | 方式 |
| --- | --- |
| 现有流程 | `context_flow`、`context_policy`、`storage_flow` 不变通过 |
| 增量/全量一致 | 在固定前缀上交替执行增量续接与全量重放，entries/state 必须一致（复用 checkpoint_projection 的测试模式） |
| 失败路径 | 取消、host prepare 失败、前缀校验失败都不推进 resume；重试仍从上一个成功边界开始 |
| 每步分配 | P0 example：改动前后的 live/peak 曲线 |
| 堆高水位 | P0 验证最大会话的堆高水位是否随步数增长；不是断言，是测量 |
| 产品验证 | 同一负载重启后观察 RSS 与 `malloc_info` 的 in-use/free 比例 |
| 兼容 | `cargo test --workspace --locked`；`xtask check` 不需要 `graph` 变更 |

不设 CI 字节阈值；数值写进 PR。

## 7. 兼容性

- 无事件、存储、协议或配置变化；`ProjectionCheckpoint`/`ProjectionCheckpointState` 的序列化不变。
- `ModelRequest` 序列化不变；冻结的事件 fixture 不动；压缩 `replacement` 和 `ContextBaselineEntry` 不改类型。
- 改动限于 capabilities 的内部类型（`HistoryEntry`、`context_messages`、`ContextPreparation` 的 prepared 槽）。
- 不改 runtime 接口：P3 使用现有 `Timeline::local`；P4 暂不实施。
- `Timeline` 的快照、压缩和读取契约不变。

## 8. 风险与备选

| 项 | 说明 |
| --- | --- |
| P3 的一致性 | 投影必须与 Timeline 已发布事实一致；resume 只在前缀校验成功后提交，失效或续接错误回退全量重建；增量与全量在固定前缀上必须一致 |
| P3 的内存 | 保留前缀 entries + `ProjectionCheckpointState`，约为上下文大小；不是历史；重载重建 |
| P3 的提交点 | 取消、host prepare 失败、前缀校验失败都不推进；下一次从上一个成功边界继续 |
| P1 的分配 | `Arc` 从 request 移到 entry，替换当前 `map(Arc::new)`，净减少一次/消息；P0 验证 |
| 备选：分配器调参 | `mallopt(M_MMAP_THRESHOLD)` 或周期 `malloc_trim` 能压 RSS，但不减少分配和 CPU；只作兜底 |
| 备选：更快压缩 | 改变上下文质量与成本，不在本计划内 |
| 未做 | 跨 Session 共享投影、历史缓存、快照频率调整、P4 分段读取 |

## 9. 规则检查

- **§4 rule 1**（改别的模块的状态要写指令事件）：不适用。投影是 branch 内由 Timeline 事实派生的状态，
  不改变其他模块的状态，不新增指令事件或结果事件。
- **rule 2**（交接后不能撤回）：不适用，交接没有变化。P3 只复用已交接事件的重放结果；resume 只在
  请求前缀校验成功后提交，取消或失败不推进。
- **rule 3**（before 只同步变换草稿）：不适用，没有 before 回调。
- **rule 4**（核心机制不带业务含义）：不适用，Timeline、Dispatcher、批次不改；投影在 capability 内。
- **rule 5**（原子批次一起提交、按序派发）：不适用，不新增批次、不写事件；续接按已提交事件的序号处理。
- **rule 6**（模型上下文只包含交接后被读取的事件）：满足。保留的 `ProjectionCheckpointState` 带有
  `inputs`、`delivered`、`prompts`、`configuration`、`open_tools`、`output`、`output_open`，
  续接与全量重放使用同一套采纳/消费规则；测试在固定前缀比较增量与全量结果。
- **rule 7**（插件只通过公开事件和服务交互）：满足。投影由 branch 的 `ContextPreparation` 拥有，
  只通过 `Timeline` 的公开读取（`local` / `rendered_context`）和自身状态工作，不读其他能力的状态。
- **rule 8**（基础设施默认不变）：满足。P3 使用现有 `Timeline::local`，不改 Timeline、Dispatcher、
  装配或生命周期；`ProjectionCheckpoint` 仍是压缩锚定的持久快照，契约未改。P4 暂不实施，因此本计划
  不提 runtime 需求；若以后实施，按 rule 8 的五项格式（被挡住的规则、考虑过的机制、通用能力、
  对序号/批次/中断/重载的影响、owner 与 issue）单独提出。
- **rule 9**（只有后续决定要用的事实才进 Timeline）：不适用，投影是内存态，不写事件。
- **ADR 002 D1–D5**：branch 作用域；不新增 wiring 模块、不新增配置项、不改生命周期；投影只由
  branch 拥有并重建，接口保持语言中立。

## 10. 删减检查

- P4 暂不实施；只有 P3 之后冷重建峰值仍然明显时才单独提出。
- P1 只把 `HistoryEntry.message` 换成 `Arc<Message>`，不改压缩 `replacement`/`ContextBaselineEntry`，
  不新增包装类型。
- P3 复用现有 `ProjectionCheckpointState`/`replay` 和 `Timeline::local`，不新增持久状态、缓存层或
  runtime 接口。
- 不新增通用内存统计接口；测量只存在于 opt-in example。
- 不为了压低 RSS 而改压缩阈值或加定时清理。
