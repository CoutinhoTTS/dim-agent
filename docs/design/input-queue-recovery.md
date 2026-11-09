# InputQueue：保存和读取恢复状态

Status: Draft.
Scope: branch InputQueue；客户端 pending 合同不变。
Issue: [#1320](https://github.com/arcships/dim/issues/1320)。
依据：[InputQueue](../architecture/rust-agent/plugins/InputQueue.md)、[状态结果](owner-state-indexed-reads.md)、[插件合同](plugin-contract.md)、[设计规则](../architecture/rust-agent/principles.md)、[ADR 002](../adr/002-plugin-scope.md)、[rutis](rutis-usage.md)。

## 1. 决定

InputQueue 保存自己的完整队列控制状态，不再通过历史 input/turn 字段重放恢复。UI pending 继续由既有方法返回；它排除 handover，不作为本恢复的数据源。

新增 owner 结果类型 `input-queue/recovered`，采用既有扩展事实路径，不新增 query/getState/saveState 接口。源输入的已完成位置遵循[状态结果协议](owner-state-indexed-reads.md)。事件是提议的合同，不表示类型已实现。

## 2. 恢复对象

结果的 schema、source_through 由上述协议定义；state 的唯一类型为 QueueState：

| 字段 | 类型与含义 |
| --- | --- |
| turn | 当前 turn ID 或 null。 |
| starting | boolean；已经释放驱动输入但未观察到 start/interrupt。 |
| entries_root | 当前队列不可变有序树的根事件 ID；空队列为 null。 |
| queued | 当前普通输入数量，不计 handover；供已发布结果直接更新 InputSnapshot。 |

Entry 只有一个定义：input_id、源 sequence 和 kind 对象。sequence 决定 Queue 输入的释放顺序，不保存另一份 branch_id。kind 对象的 kind 字段为 input 时，含有效 mode 和 released；为 handover 时含目标 turn_id，不含 released。恢复不需要 user/blocks 正文。

QueueState 的业务字段直接使用这个恢复类型；节点中的 Entry/EntryKind 在内存和持久化边界共用定义。持久化的子节点是事件 ID，装载后的子节点是同一数据的内存引用；只在持久化边界解析。

初始化计算出的欠写 Apply/Delete 与第一份结果同批提交，不另存一份长期 owed 列表。结果不含进程 waiter、任务句柄或客户端连接。

不从 applied 写入序号重新推导 starting。释放发生在源输入或 turn/end 的处理步骤，随后 interrupt 可以清除 starting，即使对应 applied 更晚才提交。

## 3. 输入与状态转移

| 源输入 | 状态和输出 |
| --- | --- |
| input/message | 按既有默认 mode 与 handover 规则加入、释放或删除。 |
| input/consumed | 移除对应条目。 |
| input/withdraw | 仅撤回仍被本 owner 持有、未释放的条目；必要时产生 Delete。 |
| turn/start | 更新 turn，清除 starting。 |
| turn/end | 按原状态机结束 turn、删除对应 handover、释放下一项。 |
| stop/interrupt | 清除 starting；不凭空启动 held queue。 |

input/applied 和 input/deleted 是本 owner 的输出，不再次改变已经完成的同一次转移。已提交输出不能在恢复时重新写；其它生产者不得用这些输出修改队列的私有状态。旧格式初始化仍按原兼容规则解释它们。

## 4. 写入和恢复

```text
源事件 -> InputQueue 按序 handler
          -> 从当前状态计算下一状态及 Apply/Delete
          -> append 原子批次：[Apply/Delete，新队列节点，input-queue/recovered]
          -> handler 返回，不等待数据库
批次持久化 -> Dispatcher 送达 recovered
             -> 发布 queued 数；撤回回复等待批次发布后返回

恢复 -> 最后一个恢复状态，按身份装载当前根可达的节点
     -> 按类型和位置索引读取 source_through 之后的源输入
     -> 仅处理这些输入，提交输出和新状态
```

处理一批源事件时，转移仍逐项按序执行；可以合并最后恢复状态，不合并掉必要终态。source_through 只推进到本批已经按序完成的最后源位置。普通新结果与它产生的 Apply/Delete 一起提交，因此不留下“状态说已完成、终态尚未写”的中间持久状态。

本代处理了新的源输入，但批次尚未提交时，状态不得被当作持久结果返回。提交失败中止该代处理并走 fault；不能继续以已修改内存接受更多工作。沿用 Timeline 的同步登记和异步持久化，不新增 worker。冷恢复和撤回方法等待既有发布通知，不在 handler 内等待 SQLite。

撤回回复在批次提交后才完成。客户端 waiter 不恢复；重启后的队列状态和终态必须与已经提交的事实一致。

恢复发布最终结果后，Dispatcher 可能仍会送达恢复期间提交的中间结果。InputSnapshot 按结果自身的提交位置单调更新，忽略已发布结果之前的记录；不能把队列数字退回前一恢复页。这个发布位置只属于本代内存，不替代 source_through。

正常恢复只读一个 QueueState、当前树节点和真正未完成的源输入。它既不按历史 turn 重放，也不调用 session.input.pending 再猜 released/starting。缺兼容结果进入显式初始化，不能在普通 get 中隐藏全历史恢复。

## 5. 队列节点与读写成本

不能每次变化都复制当前完整队列。队列节点是本 owner 的私有 `input-queue/node` 事实，数据为 entry、left、right、height、queued、handovers、first_held。left/right 为子节点事件 ID 或 null；queued 为该子树的普通输入数量；handovers 为 handover 数量；first_held 为尚未释放的 Queue 输入中最小的源序号，没有则为 null。节点不可改写，按 input_id 建 AVL 树，消费、撤回和释放直接定位条目。最早 Queue 输入沿 first_held 定位；结束 turn 时跳过 handovers=0 的子树，只处理当前 handover 条目。Store 只存事实，不执行队列算法。

新增、移除或改变 released 时，只复制受影响的树路径及旋转所需节点；同一批内不保存最终根不可达的临时节点。只改 turn/starting 而不改条目时复用原根。节点与引用它的 recovered 在同一原子批次，结果放在最后；不新增节点存储服务或插件数据库依赖。

恢复用 query 按节点 ID 批量取必要字段，每批最多 128 个身份。节点查询的可见截止是结果自身的位置，因为节点可能晚于 source_through 写入；它与新源的读取范围分开。只装载当前根可达的 K 个节点，不枚举旧版本节点。节点缺失、循环或数据不兼容是恢复错误，不退回历史重放。

运行时只持有当前树；旧版本由 Timeline 保留。结果中的 queued 在生产时来自同一根，发布时不为更新队列数字重新加载节点。内存状态、节点与恢复记录不保存历史正文或另一份完整 entries 数组。

当前 K 项，恢复读取和内存随 K 增长；一次单项变化写一条平衡树路径和一个固定大小结果，不再序列化 K 项。批量删除 M 个 handover 按实际受影响项计费。分别测 K=0/10/1,000 的节点数、结果字节、WAL 与进程写入；不能把根变小就称写入免费。历史 N 增长不改变当前根可达的节点数。

旧程序可能在最后结果之后写出没有 recovered 的 applied/deleted。恢复对本批计算出的必要输出按 input_id 点查现有终态，仅提交尚不存在的输出，不能重复应用。查询使用原输入关联索引和固定源截止，不扫描全部终态；实时 handler 不增加该读取。此兼容行为在旧格式与恢复交错流程中验证。

## 6. 修改位置与依赖

| 位置 | 修改 |
| --- | --- |
| input_queue.rs 的 QueueState | 保留既有状态机，增加单一恢复状态类型和边界编码。 |
| InputQueue::on_event / write | 沿用按序 handler，提交原子输出/结果批次；recovered 发布后更新可见状态，不逐项独立提交。 |
| InputQueue::rebuild | 读取兼容结果和索引中的未处理输入，删除八类型历史扫描。 |
| input_queue/pending.rs | 方法及返回字段不变，由 Store 成员索引加速原查询。 |

继续依赖 Timeline、Dispatcher 和 StateCell；无新增后台任务或生命周期依赖，不依赖 Store。事件类型及源条件向机制 owner 提出的要求见下表。

| owner / 问题 | 现有合同与不足 | 需要的能力 | 位置和生命周期 |
| --- | --- | --- | --- |
| Store / #1320 | 已有类型/位置索引；恢复不得枚举旧前缀 | 将 from/through 和源类型直接用于索引范围，合并本次新输入 | 固定源截止；取消释放本页，不新增恢复成员索引。 |
| Timeline / #1320 | 已有原子批次，无需另建事务端口 | 保持必要输出与结果同批，失败不发布 | 不改序号、before、发布或关闭规则。 |

## 7. 验收、Rule check 与删减

复用队列公开流程比较撤回回复、后续 applied/deleted 和启动 turn。覆盖 queue/steer/append、handover 无目标、先释放后 interrupt 再写 applied、消费、并发输入、提交前后崩溃、初始化旧 owed 和重复恢复。固定 K、U 增加十倍已处理历史，不能读取旧源正文。

| 规则 | 本设计 |
| --- | --- |
| principles §1；ADR 002 D1 | 原 branch owner 保存队列状态。 |
| ADR 002 D2–D5 | 无接线层或新服务；无新用户配置；事件与字段是语言中立合同。 |
| §4 规则 1、2 | 不代替 Loop 消费输入，交接后不撤回。 |
| §4 规则 3、4 | 无 before I/O；Store 不执行队列状态机。 |
| §4 规则 5、6 | 输出/结果原子；不新增上下文入口。 |
| §4 规则 7、8 | 只用现有声明服务；索引需求由机制 owner 闭环。 |
| §4 规则 9 | 结果用于恢复未来决策，不为 UI 单独追加日志。 |

删除历史字段恢复循环与临时 owed 的全历史累积；保留当前 entries 是业务队列本身。首版不增加通用持久 map、三种 reader 或独立完成事件。
