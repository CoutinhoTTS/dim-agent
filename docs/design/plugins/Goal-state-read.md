# Goal：读取已完成的状态结果

Status: Draft.
Scope: Goal branch 恢复和 session.goal.get。
Issue: [#1320](https://github.com/arcships/dim/issues/1320)。
依据：[Goal](Goal.md)、[旧格式属性恢复](Goal-property-queries.md)、[状态结果](../owner-state-indexed-reads.md)、[设计规则](../../architecture/rust-agent/principles.md)、[ADR 002](../../adr/002-plugin-scope.md)、[插件合同](../plugin-contract.md)、[rutis](../rutis-usage.md)。

## 1. 决定与接口

保留 session.goal.get、原 GoalRead 及模型可见的 goal:state 块。get 读取 Goal 已完成的当前结果，不再运行 Q0/Q2/Q1 候选 fold；不新增 getGoalState 或 Goal 数据库。

goal:state 是已有完整状态，不重新发明第二个模型可见状态。新增 `goal/recovered` 只保存恢复结果及处理确认，既有事实仍足以显式重建；其 state 不进入模型。它遵循[状态结果协议](../owner-state-indexed-reads.md)。本文件是替代正常候选读取的提案，不把旧格式修复当作已实现的新接口。

## 2. 保存对象

GoalRecoveryState 是唯一恢复类型：

| 字段组 | 内容 |
| --- | --- |
| goal | 完整 Goal 状态：objective、状态及其 reason/summary、judge；无目标为 null。对外按原合同产生 GoalView，不另存一份同义 view。 |
| ordering | 当前 active、last_operator、last_change、last_block、state_after、record_after 和本地 opening 身份。 |
| turn | 当前 start/open 状态、最近 end 身份与所属 start、必要 outcome、continuation_driven。 |
| continuation | 当前 count、idle，以及尚待消费的 driving 输入身份。 |
| outstanding | 未结束 task、未结算 evaluator、当前请求/结论、尚需执行的决定及其去重身份；不保存运行时任务句柄。 |
| keys | 当前三个 Goal key 的存活标志、继承校正状态、当前有效模型可见状态记录身份。 |
| committed_outputs | 已同批提交、尚待按序消费的 Goal 输出 ID 及其决定源身份；消费后移除。 |

字段语义沿用 Goal，不复制其历史定义。不得保存历史 events、全部 continuation、全部 deletion、全部已结束 evaluator 或旧 blocks。当前结论文本仍受原长度合同限制。bound waiter、settings 值、任务句柄不持久化。

## 3. 写入与未处理输入

源类型沿用 Goal 已声明的事实；恢复结果不是它自己的新驱动输入。现有 state record/trigger 原子批次保留。Goal 把本次完成的状态、必要的校正输出和处理输入身份同批写入，顺序与确认遵循状态结果协议。

Goal 的模型可见自写输入统一作为后续源输入消费，不提前推进 source_through。本批 state 记录已经处理的源前缀，以及 committed_outputs 中已经提交的输出 ID；不假设输出与决定源之间没有其它输入。后续按原顺序消费这些输出后再移除 committed_outputs。外部提交的 Goal 触发也按其原始位置处理，不用过滤后的候选顺序代替。

处理普通对话输入时，只有原 Goal 规则需要的控制字段进入 state；没有 Goal 不收集正文。连续无状态变化的处理确认可以合并；遇到需决定的 turn/end、interrupt 或删除，必须按原顺序完成决定，不把后面的状态当作当时状态。

## 4. get 与冷恢复

```text
session.goal.get
  -> 沿既有 owner 路由等待当前处理完成
  -> 取得同一结果的 goal 与 source_through，产生原 GoalView
  -> 返回原 GoalRead

冷恢复
  -> 最后结果，检查 schema 后取得 GoalRecoveryState
  -> 类型/位置索引中 source_through 之后的 Goal 源输入
  -> 结算真实欠写工作，提交结果
```

get 没有调用方 through，不增加任意历史版本 API。入口固定目标 S，并用索引取得 S 内最后一个相关源位置；等待结果覆盖这个源位置，且 S 内会影响目标状态的 committed_outputs 已消费。它不等待未来 continuation 或尚未产生输出的轮次决定。私有结果事实不属于相关源，不为追上它们再生成无穷结果。返回位置由该有效结果和已经确认没有相关源缺口的目标确定，不能仅把 raw head 填入回复；等待期间不自己扫描候选。结果中只选 goal 字段，由 Goal 生成原 GoalView。

结束分支没有 owner 时，通过已有 TreeCommands.query 取得最终兼容结果的 view；结果不存在只允许显式旧格式初始化。不能把“最后一个未删除 goal:state 块”当作同等结果：已提交 deletion 而补偿未提交的前缀会使它过期。

## 5. 删除与旧数据

删除是仍需结算的业务输入，不是一次 getter 查询。Goal 先使旧结果对该删除不再就绪；处理删除及补偿后才推进 source_through。发生崩溃时，删除仍位于旧 source_through 之后，恢复不能把旧 view 宣布为最新。

重算只允许处理实际受影响的 Goal 变更、记录与决定，不扫描普通对话正文。通过删除输入 ID 点查原始输入、引用该输入的 after/trigger 关系，定位最后未受影响的状态边界；对确实依赖被删输入的后续操作重算。受影响操作数 D 可以很大，必须报告，不能承诺任意撤销为常数时间。具体依赖字段索引须由 Goal 与存储 owner 确认，不能在 SQL 中解析标记。

旧 key、未配对记录及缺少结果的旧库用原 Goal 语义初始化一次。Q1/Q2 和历史 Fold 只属于这条初始化/等价路径，不能因新索引缺失在每次 get 隐藏调用。首次初始化的成本与已完成初始化后的 resume 分开验收。

## 6. 修改位置与机制需求

| 位置 | 修改 |
| --- | --- |
| read.rs::current | 正常路径只取有效结果的 goal，产生原 GoalView，删除 Q2/Q1 页面循环。 |
| session.rs get | 沿既有 Owner 路由等待真实覆盖位置，不将 write_guard 视作处理屏障。 |
| branch.rs / restore.rs | 使用 GoalRecoveryState 和已保存前缀之后的新源恢复；旧属性恢复保留为显式初始化入口。 |
| Fold | 保留业务转移，恢复编码只含上表的当前状态；删除结果读取端的历史容器。 |

| owner / 问题 | 现有机制与限制 | 需求 | 位置及生命周期 |
| --- | --- | --- | --- |
| Store / #1320 | 已有类型/位置索引，恢复不能扫描全部 Goal 候选 | 按源类型/公开属性条件，在 source_through 之后定位新源，不解释 Goal 文本 | 分支、本地 opening 和固定源截止不得丢失。 |
| Goal / #1320 | 删除可能使已发布块过期 | 定义结构化依赖身份和受影响边界，维持原删除结果 | 处理完成才确认；取消不发布部分新状态。 |

## 7. Rule check、验收与删减

复用 Goal get、变更、删除、续行和 evaluator 公开流程；验证丢失补偿、迟到写、旧 key、fork opening、并发 get 与恢复、重复输入和关闭。固定 view 与未处理数，增加十倍旧对话/Goal 历史，get 不调用候选 Fold。删除单独记录 D 和补偿成本。

| 规则 | 本设计 |
| --- | --- |
| principles §1；ADR 002 D1 | 状态和任务仍归 Goal branch。 |
| ADR 002 D2–D5 | 原 Session 路由不变，无接线服务或新配置；结果事件定义为可序列化数据。 |
| §4 规则 1、2 | 不替 InputQueue 交接、消费或删除输入。 |
| §4 规则 3、4 | 不在 before 重算；Store 不解释 Goal。 |
| §4 规则 5、6 | 必需输出与确认同批，goal:state 仍走原采用路径。 |
| §4 规则 7、8 | 继续声明原服务依赖，缺索引由机制 owner 实现。 |
| §4 规则 9 | 恢复结果用于未来执行与删除处理，非单纯 UI 日志。 |

不新增 Goal CRUD 服务或第二套文本状态。Fold 直接持有 GoalRecoveryState 的业务字段，当前目标只定义一次；get 与恢复选择不同字段，不各自 fold 历史。
