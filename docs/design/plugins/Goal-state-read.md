# Goal：读取已完成的状态结果

Status: Draft.
Scope: Goal branch 恢复和 session.goal.get。
Issue: [#1320](https://github.com/arcships/dim/issues/1320)。
依据：[Goal](Goal.md)、[旧格式属性恢复](Goal-property-queries.md)、[状态结果](../owner-state-indexed-reads.md)、[设计规则](../../architecture/rust-agent/principles.md)、[ADR 002](../../adr/002-plugin-scope.md)、[插件合同](../plugin-contract.md)、[rutis](../rutis-usage.md)。

## 1. 决定与接口

保留 session.goal.get、原 GoalRead 及模型可见的 goal:state 块。get 读取 Goal 已完成的当前结果，不再运行 Q0/Q2/Q1 候选 fold；不新增 getGoalState 或 Goal 数据库。

goal:state 是已有完整状态，不重新发明第二个模型可见状态。新增 `goal/recovered` 只保存恢复结果及处理确认，既有事实仍足以显式重建；其 state 不进入模型。它遵循[状态结果协议](../owner-state-indexed-reads.md)。本文件是替代正常候选读取的提案，不把旧格式修复当作已实现的新接口。

## 2. 保存对象

结果采用以下形状；业务状态沿用现有 Fold，序列化时只写当前字段，不新建另一套目标模型：

```json
{
  "schema": "goal/v1",
  "source_through": 9980,
  "input_id": "本次真实处理的 InputMessage ID，或 null",
  "before": "处理这个输入之前的结果 ID，或 null",
  "state": {
    "opening": 1,
    "fold": {},
    "driving_root": "当前驱动输入树根 ID，或 null"
  }
}
```

opening 为本地起点，没有 opening 的兼容历史用 null。input_id/before 只服务输入删除定位；非 InputMessage 的结果两者均为 null。before 可以覆盖一个更早的完整前缀，不要求恰好是前一条原始事件。首次初始化没有逐输入边界，不伪造 before。

| 字段组 | 内容 |
| --- | --- |
| goal | objective、state、judge；objective=null 表示无目标。state 用 kind 标记，paused/completed/blocked 的文字为 text；judge=model/evaluator。对外按原合同产生 GoalView，不另存一份同义 view。 |
| ordering | 当前 active、last_operator、last_change、last_block、state_after、record_after 和本地 opening 身份。 |
| turn | 当前 start/open 状态、最近 end 身份与所属 start、必要 outcome、continuation_driven。 |
| continuation | continuation_count、continuation_idle、尚待消费的 continuation_inputs；driving 成员由 driving_root 表示。 |
| outstanding | 未结束 task、未结算 evaluator、当前请求/结论、尚需执行的决定及其去重身份；不保存运行时任务句柄。 |
| keys | 当前三个 Goal key 的存活标志、继承校正状态和本地 Goal 块数量。 |
| pairing | 至多一个 held_record，仍按原始紧邻位置和 submission ID 配对。 |

字段语义沿用 Goal，不复制其历史定义。不得保存历史 events、全部 continuation、全部 deletion、全部已结束 evaluator 或旧 blocks。当前结论文本仍受原长度合同限制。bound waiter、settings 值、任务句柄不持久化。

Fold 的 requests 最多保留最近结束和当前打开轮次各一项；evaluations/conclusions/ended/failed/decided 沿原 compact/needs_decision 清理。invalidated_after 仅是删除期间的临时集合，不编码。written/restated/last_write 是本代写任务的协调信息，不作为历史去重集合持久化。

### 2.1 驱动输入树

新增私有事实 goal/driving-node：`{key, left, right, height}`。key 是驱动输入 ID，left/right 是子节点事实 ID 或 null；节点按 key 排序并保持 AVL 平衡。增删只复制树路径；仅把最终根可达的新节点与 recovered 同批提交，旧路径在内存释放。

恢复按结果位置读取当前根可达节点，每批最多 128 个 ID，检查缺失、重复、循环、排序和高度。get 不展开该树。树防止每次输入都复制完整 driving 集合；key 不冒用关联字段 input_id，以免树节点被当作输入处理结果。结果和节点均不进入模型或 FTS。

## 3. 写入与未处理输入

源类型沿用 Goal 已声明的事实；恢复结果不是它自己的新驱动输入。现有 state record/trigger 原子批次保留。驱动树的新节点、当前状态和处理位置同批保存；本次源输入的纯状态转移据此完成。续行、自动 pause、evaluator 和删除补偿仍是 Goal 自己的待结算工作，是否欠写由恢复状态中的 turn、change、conclusion 和 decided 决定，不凭 source_through 宣布这些任务已结束。

Goal 的模型可见自写输入统一作为后续源输入消费，不提前推进 source_through。冷恢复先消费固定截止内已提交的 self-output，再登记欠写决定；因此不增加输出确认数组。输出已提交而其后恢复结果未提交时，后缀消费会先看到该输出，不能在后缀消费完成前重复登记决定。删除期间则阻止保存新结果，直到校正输入被消费。外部提交的 Goal 触发也按其原始位置处理，不用过滤后的候选顺序代替。

处理普通对话输入时，只有原 Goal 规则需要的控制字段进入 state；没有 Goal 不收集正文。连续无状态变化的处理确认可以合并；遇到需决定的 turn/end、interrupt 或删除，必须按原顺序完成决定，不把后面的状态当作当时状态。

普通 handler 同步推进状态并登记批次，不等待 I/O，不新增常驻 worker。等待分支激活的任务同时响应本代取消；恢复失败时先取消任务再清理，不能等一个永远不会激活的分支。异步决定继续使用已有 owned task。没有任何源的新分支不写空结果，不能抢占 branch/start 的位置。

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

get 没有调用方 through，不增加任意历史版本 API。入口固定提交位置 S，沿已有 Dispatcher 进度确认 S 前的源已经交付，再固定这次读取要等的处理位置。后续事件到来不持续抬高本次目标。owner 的路由键是 Session ID 和 branch ID 的组合；不同 Session 的 main 不能共享或覆盖路由。Session 方法和 goal_update 使用本 Session 的身份，不从调用参数取得另一 Session 的身份。活 owner 不为 getter 查询数据库；只从已提交结果的 objective/state/judge 生成 GoalView。每条结果均可推进发布状态，不能只发布“此刻最后登记”的结果而让持续输入导致 getter 饥饿。返回位置覆盖已确认的源前缀及入口范围，不能只填入未经确认的 raw head。它不等待未来 continuation 或尚未产生输出的轮次决定。

结束分支没有 owner 时，通过已有 TreeCommands.query 取得最后结果，并以其 source_through 为下界检查是否还有新源；有缺口报告未就绪。没有结果的旧结束分支使用明确的旧格式读取路径，费用单列。不能把“最后一个未删除 goal:state 块”当作同等结果：已提交 deletion 而补偿未提交的前缀会使它过期。

### 4.1 新源的字段

身份页每页最多 128 项，返回 ID、位置、类型；范围是保存前缀之后到固定截止。没有 U 就不执行内容读取。

| 源 | 内容读取 |
| --- | --- |
| input/message | submission ID；按该 ID 判断 operator、driving、Goal 候选是否成立。只有 Goal 候选读取 blocks。 |
| input/consumed | input_id。 |
| branch/create | source、branch，不读 prompt。 |
| branch/ended、branch/create-failed | branch，不读错误正文。 |
| turn/start | 身份页的位置。 |
| turn/end | turn_id、status，不读 reason/error。 |
| stop/interrupt | scope。 |
| exec/started、exec/ended | task_id，不读命令和输出。 |
| input/deleted | §5 的删除工作。 |

operator 是 user 存在且首块 text 不以 `<dim-` 开头；driving 是 queue/steer，或 mode 缺失且 user 存在。新普通 InputMessage 需要一次 submission 查询和三次按 ID 的 Exists；Goal 输入再多一次 blocks 查询。不把 user 正文传给 Goal，不创建缺字段的 AgentEvent；实时源和字段源使用相同 Fold 转移。

## 5. 删除与旧数据

删除是仍需结算的业务输入，不是一次 getter 查询。Goal 先使旧结果对该删除不再就绪；处理删除及补偿后才推进 source_through。发生崩溃时，删除仍位于旧 source_through 之后，恢复不能把旧 view 宣布为最新。

删除按以下请求定位，不扫描普通对话正文：

```text
input/deleted.input_id
  -> 事件 ID 索引：原输入位置
  -> kind=goal/recovered AND data.input_id=原 ID，Last
  -> before 指向更早结果，按结果 ID 读取
  -> 只重算该前缀之后、固定截止以内的 Goal 控制源
  -> 补偿消费完成 -> 保存新结果
```

结果关联复用已有类型/input_id/位置索引；before 复用事件 ID 索引。不遍历结果链，也不在 SQL 中解析 Goal 标记。

重算范围可能暴露更早的其它删除。先按类型/位置取得这些删除的 ID，点查原输入与其 before；有必要时继续向前定位。每次只查新暴露的删除区间，不反复读取已检查的页面。确定边界后，建立本次输入 ID 和 after 失效位置的临时集合，再按原顺序处理边界后的控制源。被删输入不读 blocks；普通输入只做 §4.1 的控制判断。补偿候选仅来自这些被删输入，不查询所有 Goal 候选来找本次撤回。

重算时，after 失效集合包含原输入位置和区间内每个 input/deleted 的位置。前者排除已删除变更的状态记录；后者排除之前删除生成的补偿记录。restated=true 也只修复模型可见块，不作为新的业务状态转移。否则，先删除 A、再删除 B 时，第一次删除留下的“当前为 B”副本会错误地恢复 B。仍维护记录水位和块是否存活；临时集合在本次重算结束后释放，不存进结果。

D 是边界后的 Goal 控制源数，还须计入边界处的驱动树节点数和实际新正文体积。删除很早的输入可能需要长区间；分页不使这项工作变成常数，get 不承担这些重算。

旧输入从未记录 before 时，使用明确的旧格式删除重建；它不能被称为索引点查。缺失被引用结果、坏节点、未知 schema 和非法位置均报错，不进入兼容路径。首次初始化、旧边界删除与新边界删除分别验收。

旧 key、未配对记录及缺少结果的旧库用原 Goal 语义初始化一次。Q1/Q2 和历史 Fold 只属于这条初始化/等价路径，不能因新索引缺失在每次 get 隐藏调用。首次初始化的成本与已完成初始化后的 resume 分开验收。

## 6. 修改位置与机制需求

| 位置 | 修改 |
| --- | --- |
| read.rs::current | 正常路径只取有效结果的 goal，产生原 GoalView，删除 Q2/Q1 页面循环。 |
| session.rs get | 沿既有 Owner 路由等待真实覆盖位置，不将 write_guard 视作处理屏障。 |
| branch.rs / restore.rs | 使用 GoalRecoveryState 和已保存前缀之后的新源恢复；旧属性恢复保留为显式初始化入口。 |
| Fold | 保留业务转移，恢复编码只含上表的当前状态；删除结果读取端的历史容器。 |
| driving.rs / recovery/nodes.rs | 当前驱动集合的路径更新、节点同批写入和按 ID 恢复。 |
| branch/recovery/deletion.rs | 输入关联、before 边界、新暴露删除区间和补偿候选。 |

| owner / 问题 | 现有机制与限制 | 需求 | 位置及生命周期 |
| --- | --- | --- | --- |
| Store / #1320 | 已有类型/位置索引，恢复不能扫描全部 Goal 候选 | 按源类型/公开属性条件，在 source_through 之后定位新源，不解释 Goal 文本 | 分支、本地 opening 和固定源截止不得丢失。 |
| Goal / #1320 | 删除可能使已发布块过期 | 由结果的 input_id/before 定位；复用已有索引和按 ID 读取，维持原删除结果 | 处理完成才确认；取消不发布部分新状态。 |

## 7. Rule check、验收与删减

复用 Goal get、变更、删除、续行和 evaluator 公开流程；验证丢失补偿、迟到写、旧 key、fork opening、并发 get 与恢复、重复输入和关闭。固定 view 与未处理数，增加十倍旧对话/Goal 历史，get 不调用候选 Fold。删除单独记录 D 和补偿成本。覆盖连续删除 A/B、重复删除同一输入、重启后不复活旧补偿，以及 get 无数据库读取。

| 规则 | 本设计 |
| --- | --- |
| principles §1；ADR 002 D1 | 状态和任务仍归 Goal branch。 |
| ADR 002 D2–D5 | 原 Session 路由不变，无接线服务或新配置；结果事件定义为可序列化数据。 |
| §4 规则 1、2 | 不替 InputQueue 交接、消费或删除输入。 |
| §4 规则 3、4 | 不在 before 重算；Store 不解释 Goal。 |
| §4 规则 5、6 | 必需输出与确认同批，goal:state 仍走原采用路径。 |
| §4 规则 7、8 | 继续声明原服务依赖，缺索引由机制 owner 实现。 |
| §4 规则 9 | 恢复结果用于未来执行与删除处理，非单纯 UI 日志。 |

不新增 Goal CRUD 服务或第二套文本状态。Fold 直接持有业务字段；get 与恢复选择不同字段，不各自 fold 历史。不保存通用 previous-result 链、完整 driving 数组或逐输出确认集合；before 只服务真实输入的删除，驱动树只服务当前成员。两者的节点/结果字节、WAL 和额外写入均须测量。
