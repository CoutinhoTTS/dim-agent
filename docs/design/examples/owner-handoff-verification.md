# Owner 交接：消费前提与跨代结算实验

对应 #1732 的 D，代码基线 `c8e4edd5b6cf01936ec1478b8d000f5476da7a1f`。
这是实验和源码对应，不新增插件规则。A/B/C 分别在 PR #1734/#1737/#1739；本批独立基于 main，
不把尚未合并的模型当运行时依赖。运行范围、哈希和可达窗口见[记录](capability-models/owner-handoff-results.md)。

## 1. 从合同到两个交接

| 现行约定 | 实现与模型 | 前提的提供者及范围 |
| --- | --- | --- |
| [时间总线 §2、§4](../../architecture/rust-agent/time-bus.md)：生产方只按自身已消费状态复核；消费者拒绝中断后的迟到请求 | `AgentLoop::on_event` 在模型终态上写 tool-call；ToolExecutor 的 handler 记录 interrupt、拒绝批次；`InterruptHandoff.Deliver` | Dispatcher 单事实串行交付；允许在较早事实尚在派发时追加 interrupt。两个消费者进度分开，不引入全局取消位 |
| ToolExecutor 在途工作先释放资源再写结果；Loop 等结果再结束 turn | [ToolExecution](../../architecture/rust-agent/plugins/ToolExecution.md) 的任务 join 与 result；`Invoke/Finish`、`TurnEndWaitsForResult` | 本模型将 host 结束作为动作机会；不证明 host 尊重取消或最终返回 |
| [重载协议](../../architecture/rust-agent/single-tree.md#能力重载协议)：旧工作不重执行，各 owner 写自己的终态 | `WaitBranch` cleanup、ToolExecutor cleanup/rebuild、Loop rebuild/on_event；`GenerationHandoff` | 一个已持久化的 turn 和顶层 batch；一个调用、各 owner 最多一次正常换代，另允许一次进程崩溃 |
| [Wait](../../architecture/rust-agent/plugins/Wait.md) 重载拒绝旧调用，并把 reload 交给工具结算 | `ReplyReload/Settle/ConsumeLoop`，`ReloadDoesNotContinue` | `Reloading` → HostError → result settlement 标记的转换来自代码；普通完成另有 `NormalReply`，不会预先假定所有结果都是重载失败 |
| 新代读完整固定 cut 后再消费后缀 | ToolExecutor `rebuild`、Loop `rebuild`；`CaptureTool/CaptureLoop` 与后续处理 | C 的暂存/串行算法作为既有接口前提；本模型不复制队列，而让 Loop 在重建前后读到终态 |
| 失败不能伪装成功接管 | `RebuildTool(FALSE)/RebuildLoop(FALSE)` 保留 failed；`Settle(FALSE)`、`StorageFailure` 保留写入故障 | 查询失败与存储故障分开；required apply 失败的装配回滚不在模型中，见第 4 节。不能据此宣称分支继续健康运行 |

源码入口：
[Loop](../../../crates/dim-agent-capabilities/src/agent_loop.rs)、
[ToolExecutor](../../../crates/dim-agent-capabilities/src/tool_executor.rs)、
[Wait](../../../crates/dim-agent-capabilities/src/wait.rs)、
[Dispatcher](../../../crates/dim-agent-runtime/src/core/dispatcher.rs)、
[Timeline](../../../crates/dim-agent-runtime/src/core/timeline.rs)。

## 2. C 的 cut 前提，哪些 owner 实际提供了

源码检索范围为 `crates/` 的生产 `Dispatcher.consume` 调用，共 13 处；另核对三个外语 fixture。
这不是所有读历史的插件清单，也不把仅 `subscribe/listen` 的 live-only 接口混进来。

令注册时已派发位置为 d，之后回调读取的 allocated head 为 h。
`Dispatcher.consume` 先 subscribe 再调用 rebuild；`Timeline.committed()` 在锁内返回 allocated，
`Dispatcher.committed()` 委托同一读。因此在同一条仍存在的 Timeline 上有 d ≤ published ≤ h。
下表各回调在注册之后取 h，而非使用装载前保存的旧 cut。
**这闭合的是 cut 不过旧；查询选择是否重建了正确业务状态，是另一项义务。**

| Owner / 源码 | cut、查询与返回 | 本轮核对结论及业务边界 |
| --- | --- | --- |
| [Loop](../../../crates/dim-agent-capabilities/src/agent_loop.rs) `rebuild` | 开头 committed；branch 固定 through、include_publishing；分页不改 through；返回原 through | 包含旧代尚未发布终态；未结束 turn 标记 failed，等待所属 owner 的终态；未重新证明全部请求恢复选择 |
| [ToolExecutor](../../../crates/dim-agent-capabilities/src/tool_executor.rs) `rebuild` | 同上；先选最新控制，再选无关联结果的 batch | 只补失败结果；读子结果效果的附加 query 可取更新 head，以包含刚补写的 child terminal；不把这个附加读取当消费 cut |
| [ContextPreparation](../../../crates/dim-agent-capabilities/src/context_preparation.rs) `rebuild` | committed、固定 through、include_publishing、返回原 through | 未交给 model/call 的准备归本 owner；完整替换/压缩选择仍复用原局部模型和流程 |
| [ModelProvider](../../../crates/dim-agent-capabilities/src/model_provider.rs) `rebuild` | committed、固定 through、include_publishing、返回原 through | model/call 的终态归本 owner；不把 retry 的本代控制状态迁移给新请求 |
| [Approval](../../../crates/dim-agent-capabilities/src/approval.rs) `rebuild` | committed、固定 through、include_publishing、分页、返回原 through | 补取消未结算审批；历史依赖多于 live 的 approval/respond |
| [Permission](../../../crates/dim-agent-capabilities/src/permission.rs) `rebuild` | committed、chain 含 publishing，取最后 policy，返回 reply.through | 祖先策略与本分支版本分开；没有把所有历史事件当实时重放 |
| [Wait](../../../crates/dim-agent-capabilities/src/wait.rs) `rebuild` | committed、branch 含 publishing，取最后 turn 边界，返回 reply.through | 只重建 open，不恢复旧通知或旧 wait future |
| [InputQueue](../../../crates/dim-agent-capabilities/src/input_queue/recovery.rs) `rebuild` | 先 committed，再 await_publication；固定 through、不含 publishing；返回原 through | 等的是捕获的 h，不用后来移动的 head；等待失败返回错误 |
| [BackgroundExec](../../../crates/dim-agent-capabilities/src/workspace_tools/background.rs) `rebuild` | committed、固定 through、query 含 publishing、返回原 through | orphan 结算和缺失 wake 选择各有合同；repost 的错误处理不由本轮 cut 检查证明 |
| [Goal](../../../crates/dim-goal/src/branch.rs) `rebuild` | committed、publication(h)、load 固定 h；返回 h | [read](../../../crates/dim-goal/src/read.rs) 读取最后 goal/state；[query](../../../crates/dim-goal/src/branch/recovery.rs) 不含 publishing；pause_reloaded，不恢复旧任务 |
| [CollaborationTimeline](../../../crates/dim-app-server/src/collaboration.rs) `restore` | 回调内 committed、固定 through、include_publishing；返回原 through | 整理旧操作；部分撤回工作另等 activated；这不等于所有协作副作用已经结束 |
| [HooksBranch](../../../crates/dim-agent-capabilities/src/hooks/runner.rs) apply 回调 | 回调内 committed，设置 view.last，返回 h；不读历史 | [Hooks 合同](../../architecture/rust-agent/plugins/Hooks.md) 明确不重跑旧 hook；不是遗漏了通用恢复扫描 |
| [BranchLine](../../../crates/dim-agent-capabilities/src/timeline_tree.rs) apply 回调 | 回调内 committed，返回 h；随后 want Roster/Parent | [BranchTool 合同](../../architecture/rust-agent/plugins/BranchTool.md) 从当前目录和按需历史重建；跳过旧触发不等于恢复整个业务视图 |

[TS branch](../../../runtime/fixtures/branch-part.ts)、[TS extension](../../../runtime/fixtures/extension-events.ts)、
[Python branch](../../../runtime/fixtures/python/python_branch_part.py) 均在 rebuild 回调里调用
`dispatcher.committed()`，以同一 cut 查询 include_publishing 后返回。它们是示例证据，不能替第三方插件作保证。
新插件仍必须自己提供这个前提；无需枚举封闭消费者名单。

## 3. 中断与迟到请求

[InterruptHandoff](capability-models/InterruptHandoff.tla) 保存 log、published、共享串行派发位置，
以及 Loop/ToolExecutor 各自已消费位置和中断状态。一个事实可先送达任一 owner，但必须都处理完才开始下一事实。
后台发布、外部 interrupt、工具调用的开始/完成可以夹在两次处理之间。

- `Model` 是该 turn 的模型输出完成，不是再次发起模型；`Deliver("loop")` 根据 Loop 自己的已消费中断决定写请求或结束。
- interrupt 可以先提交，Loop 仍在处理更早的完成事件，因而写出排在 interrupt 后面的请求；`late` 记录这个窗口，**不阻止它发生**。
- ToolExecutor 消费 interrupt 后取消当前工作；读到迟到请求时写未执行结果。已经开始的工作可晚于 interrupt 完成。
- `Invoke` 代表取消检查之后进入工具；`Finish` 代表其任务退出和结果提交，外部资源释放是 host 的前提。
- 一个 turn 一项顶层 batch；第二配置允许下一 turn，检查 turn 边界清除中断后新工作仍能运行。下一 turn 在前一 end 派发完后开始，省略同一 end 回调内提前追加 start 的窗口。

检查：迟到请求不启动、被拒请求已有结果、已交出 batch 的 turn 不先结束、消费位置保持前缀；
动作性质检查生产方使用本地消费状态、工具不在本地已取消后启动。没有假设调度公平或声称最终终止。

## 4. 旧工作、终态与新代

[GenerationHandoff](capability-models/GenerationHandoff.tla) 分开 Wait、ToolExecutor、Loop 的阶段，
并区分 append、durable、published。状态名 old/new 是模型代号，不是产品的新增生命周期。

- `CloseWait → ReplyReload → RemoveWait`：旧调用释放后才结束 Wait 换代；ToolExecutor 可以稍后才写结果。
- `CloseTool → CancelToolCall → Settle → RemoveTool`：旧工具任务完成结算尝试后才退出；写失败仍可能完成卸载，但保留 cleanupError，绝不将“卸载完成”算作“终态成功”。
- `CaptureTool → RebuildTool`：读本 owner 固定头，无结果才补失败；不重新 Invoke。
- `ReloadLoop → CaptureLoop → RebuildLoop`：若已看到结果就结算旧 turn；否则等待其后实时结果。Loop 与 ToolExecutor 的读取顺序不限，旧终态可以仍未发布。
- `NormalReply` 是正常 Wait 返回（如超时）的对照；旧 Loop 可决定继续。带 reload 标记的结果、或恢复出来的未结束 turn 则失败，不再继续旧 turn。
- `Crash` 只保留 durable 前缀，丢失句柄与旧代调用；没有执行正常 unload。新 ToolExecutor 对持久化未结束 batch 补终态。已经发生的抽象 host effect 不因崩溃回滚。
- 模型的 query/rebuild 失败只置对应 owner failed；`Settle(FALSE)` 表示终态 append 失败，`StorageFailure` 表示尾部落盘失败。这只表示查询失败没有直接设置存储故障；不表示实际装配继续健康运行。
  required capability 的 apply 失败会使 branch build 失败并触发回滚（插件合同 §2–3）；
  本模型没有装配/回滚动作，不能用另一个 owner 仍可转换来证明真实分支会继续运行。

边界：只含一个已持久化顶层调用，不覆盖未持久化 call 的丢失、嵌套工具效果合并、任意多代、所有 host 的取消实现。
`effect` 是普通工具可能产生的外部影响标志，不表示 Wait 自己修改外部系统，也不证明外部系统的行为。
`DecideAfterResult` 的正常分支记录 continue 决定和 continued 阶段，不是 turn/end。
正常工具结果通常令 Loop 发起下一个模型请求；本模型不展开该请求及最终正常 turn/end。
`EndAfterTerminal` 因而只覆盖失败/恢复结束；该决定之后不再模拟一次正常 Loop 换代。
仅在本次历史上检查结果不重复，**不承诺跨崩溃外部 exactly-once**。

Timeline 已有存储故障时，纯查询仍可能成功；模型允许不需补写的 ToolExecutor 重建返回 new，
但故障仍阻止发布/消费。new 只是该回调返回成功，不意味着分支恢复健康。
这和“一个失败的重建不能返回成功”是两个不同命题。

## 5. 实现证据、修正与消融

本轮读取已有公开流程断言，未重跑未修改的 Rust 行为测试；不能把下面的存在当作本轮测试通过：

| 流程 | 对应关系 |
| --- | --- |
| [interrupt_flow](../../../crates/dim-agent-capabilities/tests/interrupt_flow.rs) `a_tool_call_already_committed_when_the_interrupt_arrives_is_not_executed` | 真实门闩令 call.sequence > interrupt.sequence，验证执行次数为零、未执行结果与 Cancelled turn；对应 late 窗口 |
| 同文件 `interrupt_reaches_every_capability_before_native_cleanup_and_stop_waits_for_the_turn` | 中断传播与本地清理相分离 |
| [local_execution](../../../tests/integration/tests/local_execution.rs) `wait_reload_settles_the_tool_call_without_a_user_stop` | 真实 WaitBranch restart；检查 settlement 标记、Failed turn、模型调用不增加；新代仍可接受新 wait |
| 同文件 `wait_service_reload_ends_native_and_face_calls_and_refuses_old_handles` | native/face 及旧句柄边界；模型不展开 wire |
| [context_flow](../../../crates/dim-agent-capabilities/tests/context_flow.rs) `restore_preserves_each_parent_tools_own_confirmed_child_effects`、`restoring_each_preparation_handoff_settles_without_replaying_work` | 既有嵌套效果/准备交接证据，本批未扩展为组合证明 |

发现并更正的文档问题：ToolExecution、Questions 的设计/架构页及中文翻译、ToolBinding 的 rustdoc 把“重载恢复的 batch”
写成会重新执行，与当前 rebuild 和重载协议不符；同时把 Dispatcher 依赖的 “Replay batches” 改为恢复/live 交接。
只修正文档和注释，不改变执行、事件或持久化格式。

消融：不复制 A 的写入队列、B 的查询表达式、C 的订阅缓冲；不增加全局取消位。
删除冗余的 startedAfterRestart 观察量，以 Invoke 的动作性质及重建不调用 host 的性质表达边界。
保留各 owner 阶段、各自 cut、持久化/发布位置和清理失败，因为它们分别区分真实交接窗口。
所有结果都是有限模型检查；D 的两组交接不等于所有 cap 的任意组合已证明。
剩余工作包括其他 owner 的业务选择/终态义务组合、A/B/C 已列的模型边界及 facade 故障观察（#1718/#1719）。
