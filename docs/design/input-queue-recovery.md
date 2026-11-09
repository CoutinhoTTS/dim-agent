# InputQueue：保存和读取恢复状态

Status: Draft.
Scope: branch InputQueue；客户端 pending 合同不变。
Issue: [#1320](https://github.com/arcships/dim/issues/1320)。
依据：[InputQueue](../architecture/rust-agent/plugins/InputQueue.md)、[状态结果](owner-state-indexed-reads.md)、[插件合同](plugin-contract.md)、[设计规则](../architecture/rust-agent/principles.md)、[ADR 002](../adr/002-plugin-scope.md)、[rutis](rutis-usage.md)。

## 1. 决定

InputQueue 保存自己的完整队列控制状态，不再通过历史 input/turn 字段重放恢复。UI pending 继续由既有方法返回；它排除 handover，不作为本恢复的数据源。

新增 owner 结果类型 `input-queue/recovered`，采用既有扩展事实路径，不新增 query/getState/saveState 接口。源输入和结果确认遵循[状态结果协议](owner-state-indexed-reads.md)。事件是提议的合同，不表示类型或索引已实现。

## 2. 恢复对象

结果的 schema、base_through、source_through、source_ids、previous_result 由上述协议定义；state 的唯一类型为 QueueRecoveryState：

| 字段 | 类型与含义 |
| --- | --- |
| turn | 当前 turn ID 或 null。 |
| starting | boolean；已经释放驱动输入但未观察到 start/interrupt。 |
| entries | 按原位置升序的 QueueRecoveryEntry 数组。 |
| owed | 尚未同批提交的必要 Apply/Delete；正常新结果为空，初始化可包含旧代欠写项。 |

QueueRecoveryEntry 只有一个定义：input_id、输入 position，以及 kind。普通输入 kind 为 input，含有效 mode 和 released；handover kind 含目标 turn_id，不含 released。恢复不需要 user/blocks 正文。

QueueState 的业务字段直接使用这个恢复类型；旧 Entry/EntryKind 调整为同一套可序列化类型，不在内存和恢复数据中分别维护两份队列定义。

owed 项含 input_id、动作种类、Apply 的有效 mode、决定所依据的源事实 ID。它不含进程 waiter、任务句柄或客户端连接。

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
源事件 -> InputQueue 顺序任务
          -> 从当前状态计算下一状态及 Apply/Delete
          -> append 原子批次：[Apply/Delete，input-queue/recovered]
          -> 提交成功，发布 queued 数及原有撤回回复

恢复 -> 最后一个恢复状态
     -> 索引取得真实未处理输入
     -> 仅处理这些输入，提交输出和新状态
```

处理一批源事件时，转移仍逐项按序执行；可以合并最后恢复状态，不合并掉必要终态。结果的 source_ids 确认本批所有处理过的源输入。普通新结果与它产生的 Apply/Delete 一起提交，因此不留下“状态说已完成、终态尚未写”的中间持久状态。

本代处理了新的源输入，但批次尚未提交时，状态不得被当作持久结果返回。提交失败中止该代处理并走 fault；不能继续以已修改内存接受更多工作。handler 不等待 SQLite，顺序任务由 InputQueue 登记和关闭。

撤回回复在批次提交后才完成。客户端 waiter 不恢复；重启后的队列状态和终态必须与已经提交的事实一致。

正常恢复只读一个 QueueRecoveryState 和真正未完成的源输入。它既不按历史 turn 重放，也不调用 session.input.pending 再猜 released/starting。缺兼容结果进入显式初始化，不能在普通 get 中隐藏全历史恢复。

## 5. 读写成本

首版 state 直接保存当前 entries，选择这个最小结构，不预先引入队列节点服务。当前 K 个条目，恢复内存和结果写入均为 O(K)；每项只有身份、位置和控制字段。没有 pending 时不保存旧 entries。

这会增加每个实际队列变化的写入，不能宣称写入恒定。分别测 K=0/10/1,000 的结果字节和 WAL；若大队列写放大不能接受，再比较同批不可变成员节点方案，不在读取端恢复历史规避写入成本。已处理历史 N 增长不改变 K 大小的恢复对象。

## 6. 修改位置与依赖

| 位置 | 修改 |
| --- | --- |
| input_queue.rs 的 QueueState | 保留既有状态机，增加单一恢复状态类型和边界编码。 |
| InputQueue::on_event / write | 改为 owner 顺序处理和原子输出/结果批次，不逐项独立提交。 |
| InputQueue::rebuild | 读取兼容结果和索引中的未处理输入，删除八类型历史扫描。 |
| input_queue/pending.rs | 方法及返回字段不变，由 Store 成员索引加速原查询。 |

继续依赖 Timeline、Dispatcher、StateCell 和 branch lifecycle；不依赖 Store。事件类型及源条件向机制 owner 提出的要求见下表。

| owner / 问题 | 现有合同与不足 | 需要的能力 | 位置和生命周期 |
| --- | --- | --- | --- |
| Store / #1320 | 反关联可表达，但当前从全部源记录筛选；缺数组确认索引 | 支持源类型集合与 recovered.source_ids 的精确成员索引；schema 条件不可丢失 | 源截止与确认读头分别遵循状态结果协议；取消释放本页。 |
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
