# Pursuit

状态：草案，Ready 候选（[#146](https://github.com/arcships/dim/issues/146)，里程碑 M2；没有待定的产品决策，见 [§12](#12-进入-ready-还差什么)）。作用域：对 [Goal](Goal.md) 的 Session 部分与分支部分的修订，不新增 Capability（[ADR 002](../../adr/002-plugin-scope.md)）。语言：TypeScript，放在 Goal 的包 `runtime/plugins/goal` 里（[Goal §10](Goal.md#10-实现)）。依赖：Goal（P5，已由 [#199](https://github.com/arcships/dim/pull/199) 实现）、C5（[分支 TypeScript 绑定](../../../design/branch-typescript.md)）；都已交付。里程碑 M2，不在当前计划内（[路线图](../../../design/roadmap.md) D3）。

需求（[功能列表](../../../feature-list.md)，Agent 行为）：朝明确的目标持续工作，记录完成或受阻，**并且由评审者而不是干活的模型判断是否完成**。

**Pursuit 是 Goal 的一种裁判模式。** 以 `judge: "evaluator"` 设置的目标和其他 Goal 目标运行方式完全相同，续跑、预算、中断、暂停和上限都是 Goal 的。只改变一点：工作分支的某个回合结束、Goal 因为干活的模型报告目标完成而不再续跑时，Goal 创建一个**评审分支**。评审者用一个回合检查工作，并以一个结论结束：`DONE`；`CONTINUE`，附至少仍缺少的事项及证据；或 `BLOCKED`，附只有用户能提供的东西。评审回合结束时，评审分支上的 Goal 把结论作为输入写回工作分支，由工作分支上的 Goal 决定：`DONE` 使目标完成（只有评审者能做到），`CONTINUE` 变成一次续跑，`BLOCKED` 使目标受阻。本文档取代实验性设计（#17）。

## 1. 决策

| # | 决策 | 原因 |
| --- | --- | --- |
| P1 | Goal 的裁判模式，不是独立插件 | 续跑只有一个所有者；独立插件会和 Goal 在同一个 `turn/end` 上竞争，或要读 Goal 的状态（§4 规则 7） |
| P2 | 裁判由 `set` 按目标选择，记在 `set` 块上 | 它是用户对这个目标的意图，属于 Timeline 事实 |
| P3 | 在干活模型的完成报告让 Goal 停下时评审 | 只有这个点是干活的模型认为目标已完成。干活模型的 `blocked` 直接生效（需要用户处理）；上限，以及用户自己的中断或暂停，都不是完成，不启动任何东西 |
| P4 | 评审裁判下干活模型的 `completed` 是请求，状态保持 `active` | 只有评审者能使目标完成 |
| P5 | 评审者是在该 `turn/end` 处切出的子分支 | 它能看到做了什么、为什么做，在同一策略下运行，是用 `treeCommands.create` 创建的普通分支 |
| P6 | 评审者把结论作为输入写到工作分支；由那里的 Goal 决定 | 分支只读自己的事件；跨分支通信就是按 id 写输入（[分支 TypeScript 绑定](../../../design/branch-typescript.md#subscription-rebuild-and-recovery) 第 6 条）。调用方写，所有者决定 |

## 2. 用户流程

```
 user: session.goal.set {objective: "make the CLI tests pass", judge: "evaluator"}
   └─ goal active ─▶ turn ─▶ continuation ─▶ turn ...             (Goal, unchanged)
 worker: goal_update completed "all tests pass"                  (request; still active)
 turn/end N ─▶ Goal(B) does not continue ─▶ create evaluator E at N (operation goal:evaluate:N)
 E: checks, replies "... \n CONTINUE:\n - 2 tests still fail: <output>"
 E turn/end ─▶ Goal(E) writes the conclusion to B
 Goal(B):
   ├─ DONE     → completed (by evaluator)
   ├─ CONTINUE → continuation: "list may be incomplete; assess, continue, report again"
   ├─ BLOCKED  → blocked (by evaluator)
   └─ failed   → paused (evaluation_failed)
```

客户端从工作分支的事件中读取全部信息（[Goal A.3](Goal.md#a3-读取目标)）：`set` 块上的裁判、操作为 `goal:evaluate:<N>` 的 `branch/create` 所表示的评审、结论输入和 Goal 的结局。

## 3. 对 Goal 的修改

| 部分 | 修改 |
| --- | --- |
| `session.ts` | `session.goal.set` 接受 `judge?: "model" \| "evaluator"`（默认 `model`；其他值返回 `invalid_params`），在 `set` 目标块上标记 `judge="evaluator"` |
| `goal.ts` | 折叠读取 `judge`（来自 `set`，`edit` 保留）；评审裁判下，干活模型的 `completed` 块让状态保持 `active` 并记为一次请求；以及下面的决策和文本 |
| `branch.ts` | Dispatcher key 新增 `branch/create` 和 `branch/create-failed`；创建评审分支；在评审分支上承担评审角色（§4.2） |

不新增服务、注入、Settings 字段或事件类型。评审分支是普通的 host 来源子分支；它的第一条输入是提示词（[TimelineTree `first_input`](../../../../crates/dim-agent-capabilities/src/timeline_tree.rs)）。那里的 Goal 没有属于自己的目标，并像在任何子分支上一样写入起始 `clear`（[Goal §3.4](Goal.md#34-新分支清除继承的目标)），所以它不会续跑。

## 4. Goal 中的决策

每个决策都和 Goal 自己的决策一样做出：Dispatcher 处理函数依据已消费的状态，通过写入前复查的自有任务执行（[Goal A.4](Goal.md#a4-续跑决策)）。

### 4.1 工作分支

| 消费的事件 | 条件 | 动作 |
| --- | --- | --- |
| `turn/end` N | Goal 的 C1–C4 满足（目标 active、未中断、回合 `completed`、是最新的且未决定），裁判为 `evaluator`，且该回合包含干活模型的 `completed` 块 | 取代 Goal 的续跑决定：`treeCommands.create(B, N, {prompt, source: {host: {operation_id: "goal:evaluate:N"}}}, cancelled)`。`branch/create` 标记 `N` 已决定，所以重载不会再次创建 |
| `goal:evaluate:N` 的 `branch/create-failed`，或 `create` 抛出 | — | 暂停（`evaluation_failed`） |
| 评审 `N` 的结论输入 | 是 `N` 的第一份，并来自 `N` 的 `branch/create` 所记录的分支 | `done`：评审者判定的 `completed`。`blocked`：评审者判定的 `blocked`。`continue`：Goal 的续跑条件 C1 和 C6（active、未达 `maxContinuations`），然后写使用 Pursuit 文本的续跑，或暂停（`limit`）。`failed`：暂停（`evaluation_failed`） |

同一 `N` 的后续结论，或来自其他分支的结论，都不做任何决定。其他一切都是 Goal 的：评审者工作期间，工作分支的表现和任何 active 目标一样（用户输入、中断、暂停和续跑都按 Goal 的规则），结论到达时按序号顺序与期间发生的事情一起生效。

### 4.2 评审分支

Goal 从分支的第一条本地输入识别这个角色：首行为 `<dim-goal kind="evaluate" branch="B" evaluation="N">` 的用户文本，且 `B` 是本分支 `branch/start` 的 `parent`。

| 消费的事件 | 条件 | 动作 |
| --- | --- | --- |
| `turn/end` M，状态 `completed` | 回复带有结论 | 用 `treeCommands.input` 写到 `B`（Append），标记 `evaluation="N" evaluator="E"` |
| `turn/end` M，状态 `completed` | 没有结论 | 在评审分支上写一条追问（Queue，`after="M"`），要求它以三种结论之一结束 |
| `turn/end` M，状态 `failed` 或 `cancelled` | — | 向 `B` 写 `failed` |

回复取该回合最后一次模型请求：按序号顺序拼接 `turn_id` 匹配、且 `request_id` 等于该回合最后一个 `step/model-start` 的每个 `step/text` 的 `delta`（一行可能跨多条记录；更早的请求和思考不算）。结论从最后一个以 `DONE:`、`CONTINUE:` 或 `BLOCKED:` 开头的行开始，到末尾为止，所以 `CONTINUE` 清单可以跨多行；截断到 4000 字符。

重载时和 Goal 一样，对最新的 `turn/end` 再做一次决定：重启截断的评审回合以 `cancelled` 结束（终态事实），于是写出 `failed`；已写过的结论可能再写一次，工作分支只取第一份，会忽略它。

## 5. 上下文映射

标记、转义和 key 沿用 Goal（[A.5](Goal.md#a5-模型可见文本)）。`N` 是请求所在回合的 `turn/end`，`B` 是工作分支，`E` 是评审分支。模型可见文本保持英文。

| 写入 | 位置、key | 模式 | 文本 |
| --- | --- | --- | --- |
| 评审提示词 | E，第一条输入 | Queue | `<dim-goal kind="evaluate" branch="B" evaluation="N">` / `You are an evaluator. Branch B reported that its goal is completed. Only you decide whether it is. Do not continue the work, do not change files or settings, and do not call goal_update.` / 空行 / `Objective:` / `<objective>` / 空行 / `Reported result:` / `<summary>` / 空行 / `Verify the facts yourself by reading files and running checks; do not rely on the report or on earlier messages. End your reply with exactly one of:` / `DONE: <the verified result>` / `CONTINUE: followed by the items that are at least still missing, one per line, each with its evidence` / `BLOCKED: <what only the user can provide>` |
| 追问 | E，无 key | Queue | `<dim-goal kind="evaluate-again" after="M" branch="E">` / `Your reply did not end with a conclusion. End your reply with exactly one of DONE:, CONTINUE: or BLOCKED: as described above.` |
| 结论 | B，无 key | Append | `<dim-goal kind="report" result="done\|continue\|blocked\|failed" evaluation="N" evaluator="E" branch="B">` / `Evaluator report:` / `<结论，或 "the evaluator's turn did not complete">` |
| `done` | B，`goal:status` | Append | `<dim-goal kind="goal" op="completed" by="evaluator" evaluation="N" branch="B">` / `Result:` / `<result>` / 空行 / `An evaluator verified that the goal for branch B is completed. Do not continue working toward it on your own.` |
| `blocked` | B，`goal:status` | Append | `op="blocked" by="evaluator"`；`Blocker:` / `<what is needed>` / 空行 / `An evaluator found that the goal for branch B cannot proceed without the user. Do not continue working toward it on your own.` |
| `continue` | B，Goal 续跑 | Queue | `<dim-goal kind="continuation" count="k" limit="L" evaluation="N" branch="B">` / `An evaluator checked your report that the goal for branch B is completed and found it is not achieved yet. Its report above lists items that are at least still missing; the list may be incomplete. Assess yourself how far the goal still is, then continue working toward it. When you think it is achieved, call goal_update with status "completed" again.` |
| `failed` | B，`goal:status` | Append | Goal 的暂停文本，原因短语 "the evaluator could not check the result" |

评审裁判下，干活模型的 `completed` 块在折叠中表示 `active`；在结局或变更取代它之前，它一直是最新的状态块，所以重放仍与折叠一致（Goal A16）。

## 6. 规则检查

规则：[原则 §1 与 §4](../../../architecture/rust-agent/principles.md)、[ADR 002](../../adr/002-plugin-scope.md) D1–D5。

| 规则 | 本文档如何遵守 |
| --- | --- |
| §4.1 用指令事件改别人的状态 | 创建评审分支是发给 TimelineTree 的指令；结论是写给工作分支的输入，由那里的 Goal（所有者）决定 |
| §4.2 交接后不撤回 | 不撤回任何东西；重复的结论只是不做任何决定 |
| §4.3 只有 before 回调，不阻塞 | 不适用：没有 before 回调 |
| §4.4 核心不带业务含义 | 不改 Timeline、Dispatcher、TimelineTree、InputQueue 或 Loop |
| §4.5 批次 | 决定按已消费事件的序号顺序做出；每个结局是一条输入 |
| §4.6 上下文只含交接后的事件 | 干活的模型只通过已采用的输入看到结论；评审者的回复留在它的分支上 |
| §4.7 只用公开事件和服务 | Goal 只读本分支的公开事件，调用已导出的服务；没有分支读取另一个分支 |
| §4.8 向机制所有者提需求 | 无 |
| §4.9 只有后续决定要用的事实才进 Timeline | 不新增事件类型；`branch/create` 记录评审，结论输入是所有者据以决定的事实，追问是让评审者给出结论的事实 |
| §1 Capability 与 App Server 边界 | 不新增 Capability；App Server 不变 |
| ADR 002 D1 作用域 | 评审及其结局是工作分支的状态，归那里的 Goal 分支部分；评审角色是评审分支上的 Goal 分支部分 |
| ADR 002 D2 无接线模块 | 没有地方点名 Goal |
| ADR 002 D3 作用域依赖 | 与 Goal 相同 |
| ADR 002 D4 配置 | 裁判是每个目标的 Timeline 事实；没有新配置 |
| ADR 002 D5 语言中立契约 | `judge` 参数、标记和操作 id 都是数据 |

## 7. 向机制所有者提出的需求

无。

## 8. 安全

评审分支是普通子分支，有工作分支的权限策略并经过 Approval（[ADR 003](../../adr/003-approval-scope.md)）；“不要改动任何东西”只是指引，不是隔离。只有来自工作分支自己的 `branch/create` 所记录分支的结论才算数，所以操作者用伪造提示词创建的 host 分支不做任何决定。诊断记录序号和结果类型，从不记录目标或报告文本。

## 9. 验收

干活的模型和评审者都用假模型，放在 `goal.test.mjs` 和 `transport_flow/goal.rs`：

1. 评审裁判：续跑是 Goal 的。干活的模型报告 `completed`；状态保持 `active`；该回合结束时在 `N` 处创建一个带 `goal:evaluate:N` 的评审分支，不写续跑。`DONE` 得到评审者判定的 `completed`。
2. 带两行清单的 `CONTINUE` 得到结论和一次续跑（`count` + 1）；`BLOCKED` 得到 `blocked`；达到 `maxContinuations` 时 `CONTINUE` 得到暂停（`limit`）。
3. 干活模型的 `blocked`、上限暂停、用户中断和用户暂停都不创建评审分支。
4. 没有结论的评审回复在评审分支上得到一次追问，之后的结论生效。
5. 评审回合失败或取消、创建失败，得到暂停（`evaluation_failed`）。
6. 评审期间重启：没有第二个评审分支；被截断的评审回合得到 `failed`；写了两次的结论只生效一次。
7. 来自非记录评审分支的结论不做任何决定。
8. 流式输出的结论能跨 `step/text` 记录重组，并忽略更早请求中的 `DONE:`。
9. 模型裁判：Goal 的验收不变。

## 10. 以后的选项

本设计不需要；用户有需要时可以加：

| 选项 | 说明 |
| --- | --- |
| 只读或更便宜的评审者 | 需要在第一条输入之前配置分支，以及按分支收窄 Permission；两者都不存在 |
| CLI `/pursue` | CLI 还没有 `/goal` 命令；`/pursue` 就是带 `judge: "evaluator"` 的 `/goal` |

## 11. 消融复核

剩下的部分及每一部分的必要性：

| 部分 | 原因 |
| --- | --- |
| `set` 上的 `judge` | 区分哪些目标由评审者判定 |
| 评审裁判下干活模型的 `completed` 是请求 | 只有评审者能使目标完成 |
| 在停下的 `turn/end` 处创建 | 这是触发点；`branch/create` 也让重载不会重复创建 |
| Goal 分支部分中的评审角色 | 评审分支必须报告结论，而只有那个分支上的部件能读到它的回复 |
| 写给工作分支的结论输入 | 唯一的跨分支途径；由那里的 Goal 决定 |
| 只取来自记录评审分支的第一份结论 | 重载可能写两次，操作者也可能创建冒充的分支 |
| 回复没有结论时追问 | 否则评审会在没有结果的情况下结束 |
| 评审回合失败或取消、创建失败时写 `failed` | 这些是终态事实；没有它，目标会保持 active 却没有评审在进行 |

相对上一版删除了：声明和重新打开状态、截止时间与 `evaluationTimeoutSeconds`、`goal/evaluation-reported` 记录、评审期间针对用户输入的特殊规则，以及停止和退役分支的需求。

## 12. 进入 Ready 还差什么

没有待定的产品决策；本设计等待维护者确认。
