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

全库累计重放 1,605,556 个事件，按平均正文约 2.2 GB；单个最大会话约 1.5 GB，
与它所在 arena 的 1.35 GiB 峰值同一量级。

结论：

1. 重建结果确实被释放了：in-use 只有 107 MiB，`malloc_trim` 当场归还 1.2 GiB。
2. 但堆的高水位约等于**历史重放累计量**，而不是单次重建量。说明每次重建的缓冲没有复用，
   堆按累计量增长。
3. glibc 把这些空闲块留在 arena 里；动态 mmap 阈值让释放过的大块后续也走堆。
   这是保留现象，不是根因。

## 3. 现状：一次 prepare 实际造了几份

```text
Timeline::context_from
  └─ events: Vec<Arc<AgentEvent>>          整段历史物化        (1)
       └─ HistoryEntry { message: Message } 文本复制            (2)
            ├─ context_messages()           整份克隆：前缀校验   (3)
            ├─ context_messages()           整份克隆：取长度     (4)
            ├─ context_messages()           整份克隆：组请求     (5)
            └─ ModelSettings::request()     每条消息再 Arc::new  (6)
```

对应代码：

| 位置 | 行为 |
| --- | --- |
| [timeline.rs:805](../../crates/dim-agent-runtime/src/core/timeline.rs#L805) `context_from` | 从 sequence 1 的整段历史装进 `ContextHistory.events` 和 `positions` |
| [history.rs:10](../../crates/dim-agent-capabilities/src/history.rs#L10) `HistoryEntry` | `message: Message`，文本由事件复制而来 |
| [history.rs:555](../../crates/dim-agent-capabilities/src/history.rs#L555) `context_messages` | 对每条消息 `entry.message.clone()` |
| [context_preparation.rs:542](../../crates/dim-agent-capabilities/src/context_preparation.rs#L542) | 为校验前缀整份克隆 |
| [context_policy.rs:271](../../crates/dim-agent-capabilities/src/context_policy.rs#L271) | `context_messages(...).len()`，只为取长度整份克隆 |
| [context_preparation.rs:481](../../crates/dim-agent-capabilities/src/context_preparation.rs#L481) `configured_request` | 组请求再整份克隆 |
| [host.rs:99](../../crates/dim-agent-capabilities/src/host.rs#L99) `ModelSettings::request` | `messages.into_iter().map(Arc::new)`，每条消息一次分配 |

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

收益：每次 prepare 少 2–3 份整文本拷贝；前缀校验与取长度可以直接读条目。
风险：每条消息多一次 `Arc` 小分配，需要测量净收益。

### P2 去掉元数据物化

- 前缀长度由条目数算出（compaction replacement + context baseline + history 条数），不再造 Vec。
- 前缀校验按引用迭代比较 `&Message` 与 `&Arc<Message>`，不克隆。
- `configured_request` 复用同一份 `Arc<Message>` 列表，不再另造一份。

### P3 按边界重放（核心，#710）

每次 prepare 把最终投影（`Vec<Arc<Message>>` 和 resume 位置）留在 `ContextPreparation` 的内存里；
下一次从该位置读 `local(prev+1..)`，只投影新增事件。

- 状态属于 branch，可重建：重载、投影失败或边界失效时回退一次全量重建。
- 边界失效条件：压缩完成、prompt/配置 revision 变化、分支切换、输入被撤回。
- 不保留原始历史，只保留最终投影（即模型请求本来就需要的内容）；消费后仍可释放。
- 与快照的关系：快照仍是跨进程/跨装载的加速；内存投影只服务同一装载内的连续步骤。

这一步把每步分配从 O(历史) 降到 O(新增事件)，是关键修复。

### P4 冷重建分段（可选）

全量重建（首次、重载、边界失效）不看完整 `Vec<AgentEvent>`：按 256 条一段读取、投影、丢弃，
峰值是一段加输出。`Timeline::context()` 保留给导出、建分支等需要完整结果的调用方。

## 6. 验收与测量

| 项 | 方式 |
| --- | --- |
| 现有流程 | `context_flow`、`context_policy`、`storage_flow` 不变通过 |
| 每步分配 | P0 example：改动前后的 live/peak 曲线 |
| 堆高水位 | 100 步后 RSS 与 heap high-water 不随步数增长超过上下文本身 |
| 产品验证 | 同一负载重启后观察 RSS 与 `malloc_info` 的 in-use/free 比例 |
| 兼容 | `cargo test --workspace --locked`；`xtask check` 不需要 `graph` 变更 |

不设 CI 字节阈值；数值写进 PR。

## 7. 兼容性

- 无事件、存储、协议或配置变化。
- `ModelRequest` 序列化不变；冻结的事件 fixture 不动。
- 改动限于 capabilities 的内部类型与 runtime 的分段读取接口（若做 P4）。
- `Timeline` 的快照、压缩和读取契约不变。

## 8. 风险与备选

| 项 | 说明 |
| --- | --- |
| P3 的一致性 | 投影必须与 Timeline 已发布事实一致；边界失效要能回退，不能吞掉重放错误 |
| P3 的内存 | 保留的是投影（约上下文大小），不是历史；需要确认这不违背“历史不常驻” |
| P1 的小分配 | Arc 包装增加小对象；用 P0 比较净收益 |
| 备选：分配器调参 | `mallopt(M_MMAP_THRESHOLD)` 或周期 `malloc_trim` 能压 RSS，但不减少分配和 CPU；只作兜底 |
| 备选：更快压缩 | 改变上下文质量与成本，不在本计划内 |
| 未做 | 跨 Session 共享投影、历史缓存、快照频率调整 |

## 9. 规则检查

- **§4 rule 1–4**：投影是 branch 内由 Timeline 事实派生的状态，可在装载内复用、重载时重建；
  不新增持久状态，不写入不属于自己的事件。
- **rule 5**：P4 若增加 runtime 分段读取接口，属于跨模块要求，由 runtime 在自身文档中定义。
- **rule 6**：更新现有流程断言；不新增或修改事件 fixture。
- **rule 7**：复用现有分段读取与 `ProjectionCheckpoint`，不新增缓存层或内存统计服务。
- **rule 8**：向 runtime 提出“按段读取 / 投影回调”的能力要求，格式按 §4 rule 8。
- **rule 9–11**：不涉及终态事实、上下文注入路径或授权。
- **ADR 002 D1–D5**：branch 作用域；不新增 wiring 模块、不新增配置项、不改生命周期；
  接口保持语言中立。

## 10. 删减检查

- P4 只有在 P3 之后仍出现“冷重建峰值”问题才做；P3 单独就能解决稳态。
- P1/P2 不引入新类型层次：只把 `Message` 换成 `Arc<Message>`，不新增包装结构。
- 不新增通用内存统计接口；测量只存在于 opt-in example。
- 不为了压低 RSS 而改压缩阈值或加定时清理。
