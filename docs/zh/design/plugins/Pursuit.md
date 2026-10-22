# Pursuit

状态：草案，Ready 候选（[#146](https://github.com/arcships/dim/issues/146)，里程碑 M2；没有待定的产品决策，见 [§12](#12-进入-ready-还差什么)）。作用域：对 [Goal](Goal.md) 的 Session 部分与分支部分的修订，不新增 Capability（[ADR 002](../../adr/002-plugin-scope.md)）。语言：TypeScript，放在 Goal 的包 `runtime/plugins/goal` 里（[Goal §10](Goal.md#10-实现)）。依赖：Goal（P5，已由 [#199](https://github.com/arcships/dim/pull/199) 实现）、C5（[分支 TypeScript 绑定](../../../design/branch-typescript.md)）；都已交付。里程碑 M2，不在当前计划内（[路线图](../../../design/roadmap.md) D3）。

需求（[功能列表](../../../feature-list.md)，Agent 行为）：朝明确的目标持续工作，记录完成或受阻，**并且由评审者而不是干活的模型判断是否完成**。

**Pursuit 是 Goal 的一种裁判模式。** 以 `judge: "evaluator"` 设置的目标和其他 Goal 目标运行方式完全相同，续跑、预算、中断、暂停和上限都是 Goal 的。只改变一点：干活的模型报告目标完成之后，工作分支的回合结束时，Goal 不续跑，而是创建一个**评审分支**。评审者用一个回合检查工作，并以一个结论结束：`DONE`；`CONTINUE`，附至少仍缺少的事项及证据；或 `BLOCKED`，附只有用户能提供的东西。评审回合结束时，评审分支上的 Goal 把结论作为输入写回工作分支，由工作分支上的 Goal 决定：`DONE` 使目标完成（只有评审者能做到），`CONTINUE` 变成一次续跑，`BLOCKED` 使目标受阻。本文档取代实验性设计（#17）。

## 1. 决策

| # | 决策 | 原因 |
| --- | --- | --- |
| P1 | Goal 的裁判模式，不是独立插件 | 续跑只有一个所有者；独立插件会和 Goal 在同一个 `turn/end` 上竞争，或要读 Goal 的状态（§4 规则 7） |
| P2 | 裁判由 `set` 按目标选择，记在 `set` 块上 | 它是用户对这个目标的意图，属于 Timeline 事实 |
| P3 | 在干活模型的完成报告之后的回合结束时评审 | 只有这个点是干活的模型认为目标已完成。干活模型的 `blocked` 直接生效（需要用户处理）；上限，以及用户自己的中断或暂停，都不是完成，不启动任何东西 |
| P4 | 评审裁判下干活模型的 `completed` 是请求：Goal 以自己的 active 状态块回应它，所以状态保持 `active` | 只有评审者能使目标完成，而最新的状态块仍然就是状态，Goal 的折叠没有例外（[Goal A16](Goal.md#附录-e-假设)） |
| P5 | 评审者是在该 `turn/end` 处切出的子分支 | 它能看到做了什么、为什么做，在同一策略下运行，是用 `treeCommands.create` 创建的普通分支 |
| P6 | 评审者把结论作为输入写到工作分支；由那里的 Goal 决定 | 分支只读自己的事件；跨分支通信就是按 id 写输入（[分支 TypeScript 绑定](../../../design/branch-typescript.md#subscription-rebuild-and-recovery) 第 6 条）。调用方写，所有者决定 |

## 2. 用户流程

```
 user: session.goal.set {objective: "make the CLI tests pass", judge: "evaluator"}
   └─ goal active ─▶ turn ─▶ continuation ─▶ turn ...             (Goal, unchanged)
 worker: goal_update completed "all tests pass"                  (status completed)
 Goal(B): request block, status active, judge evaluator          (status active again)
 turn/end N ─▶ Goal(B) does not continue ─▶ create evaluator E at N (operation goal:evaluate:N)
 E: checks, replies "... \n CONTINUE:\n - 2 tests still fail: <output>"
 E turn/end M ─▶ Goal(E) writes the conclusion to B, then marks M reported on E
 Goal(B):
   ├─ DONE     → completed (by evaluator)
   ├─ CONTINUE → continuation: "list may be incomplete; assess, continue, report again"
   ├─ BLOCKED  → blocked (by evaluator)
   └─ failed   → paused (evaluation_failed)
```

客户端从工作分支的事件中读取全部信息（[Goal A.3](Goal.md#a3-读取目标)）：`set` 块上的裁判、作为 Goal 的 active 状态块的请求、操作为 `goal:evaluate:<N>` 的 `branch/create` 所表示的评审、结论输入和 Goal 的结局。最新的 `goal:status` 块始终就是状态。

## 3. 对 Goal 的修改

| 部分 | 修改 |
| --- | --- |
| `session.ts` | `session.goal.set` 接受 `judge?: "model" \| "evaluator"`（默认 `model`；其他值返回 `invalid_params`），在 `set` 目标块上标记 `judge="evaluator"` |
| `goal.ts` | 折叠读取 `judge`（来自 `set`，`edit` 保留），并且无例外地沿用 Goal 的规则（A16）：干活模型的 `completed` 块在 Goal 的请求块（§5）跟上之前就是 `completed`。请求块既不是 `set` 也不是 `resume`，所以不移动 `A`：计数仍是 `max(A, last_operator)` 之后的续跑数（[Goal §2](Goal.md#2-数据与状态)）。以及下面的决策和文本 |
| `branch.ts` | Dispatcher key 新增 `branch/create` 和 `branch/create-failed`；评审裁判下以请求块回应干活模型的 `completed`；创建评审分支；在评审分支上承担评审角色（§4.2） |

不新增服务、注入、Settings 字段或事件类型。评审分支是普通的 host 来源子分支；它的第一条 Queue 输入是提示词（[TimelineTree `first_input`](../../../../crates/dim-agent-capabilities/src/timeline_tree.rs)）。那里的 Goal 没有属于自己的目标，并像在任何子分支上一样写入起始 `clear`（[Goal §3.4](Goal.md#34-新分支清除继承的目标)），所以它不会续跑。

## 4. Goal 中的决策

每个决策都和 Goal 自己的决策一样做出：Dispatcher 处理函数依据已消费的状态，通过写入前复查的自有任务执行（[Goal A.4](Goal.md#a4-续跑决策)）。

### 4.1 工作分支

| 消费的事件 | 条件 | 动作 |
| --- | --- | --- |
| `input/message` R，干活模型的 `completed` 块 | 裁判为 `evaluator`，且任务写入时 `R` 仍是最新的状态块 | 写入请求块（`goal:status`，active，标记 `after="R"`）。消费请求块时，若最新的 `turn/end` 尚未 `decided`，就像消费 `input/deleted` 时一样重新对它做决定（[Goal A.4](Goal.md#a4-续跑决策)），所以在请求块之前被消费的回合结束会在请求块被消费后得到决定 |
| `turn/end` N | Goal 的 C1–C4 满足（目标 active、未中断、回合 `completed`、是最新的且未决定），且最新的状态块是一个之后没有评审的请求块 | 取代 Goal 的续跑决定：`treeCommands.create(B, N, {prompt, source: {kind: "host", operation_id: "goal:evaluate:N"}}, cancelled)`。`goal:evaluate:N` 的 `branch/create` 标记 `N` 已决定，所以重载不会再次创建。干活模型的 `blocked`、上限、中断或暂停都会取代请求块成为最新的状态块，所以它们都不创建评审分支 |
| `goal:evaluate:N` 的 `branch/create-failed` F | `N` 仍是当前评审 | 暂停（`evaluation_failed`），标记 `after="F"` |
| `create` 抛出 | 没有消费到 `goal:evaluate:N` 的 `branch/create`，且 `N` 仍是当前评审 | 暂停（`evaluation_failed`），标记 `after="N"`。已提交的创建总会记录 `branch/create`，之后是子分支或 `branch/create-failed`，由那个事实决定；因任务被取消而抛出时不写任何东西 |
| 评审 `N` 的结论输入 C | 是 `N` 的第一份结论，来自 `goal:evaluate:N` 的 `branch/create` 所记录的分支，且 `N` 仍是当前评审 | `done`：评审者判定的 `completed`。`blocked`：评审者判定的 `blocked`。`continue`：在折叠上检查 Goal 的续跑条件 C1、C5 和 C6（active、没有待处理的驱动输入、未达 `maxContinuations`），然后写使用 Pursuit 文本的续跑，或暂停（`limit`）；有待处理的驱动输入时不写任何东西，由该输入的回合结束按 Goal 的规则决定，此时报告已在上下文中。`failed`：暂停（`evaluation_failed`）。每个结局都标记 `after="C"` |
| 装载 | — | 在 Goal 自己的装载决定之后，对仍欠着的决定各做一次：最新的干活模型 `completed` 还没有请求块时写请求块；当前 `N` 的 `branch/create-failed` F 尚未 `decided` 时写暂停；第一份当前结论 C 没有标记 `after="C"` 的写入时做决定，除非它是之后有 `turn/end` 的 `continue`（那个回合的决定已经取代了它） |

只要 `N` 所评审的请求块之后没有带 key 的目标块，`N` 就是当前评审：没有 `set`、`edit`、`clear`、`pause`、`resume`、Goal 的暂停或结局，也没有干活模型新的 `completed`。同一 `N` 的后续结论、来自其他分支的结论，以及已被取代的 `N` 的结论，都不做任何决定。`N` 是否仍是当前评审，是 Goal 自己按序号的折叠：工作分支上之后的任何目标变更都是更新的决定，迟到的 `DONE` 不会覆盖它；两条失败路径中先写出暂停的一条也使 `N` 对另一条而言已被取代。`after=` 标记和 Goal 自己的写入一样，使每个决定在重载之间只做一次：结论提交之后、结局写入之前重载（`continue` 路径要等 Settings），会在那时写出结局；结局写入之后重载，不写任何东西。其他一切都是 Goal 的：评审者工作期间，请求块让目标保持 active，工作分支的表现和任何 active 目标一样（用户输入、中断、暂停和续跑都按 Goal 的规则）。C5 让 `continue` 不会在用户回合结束时已写出的续跑之上再叠加一次续跑。

### 4.2 评审分支

Goal 从分支的第一条本地 Queue 输入识别这个角色（Prompt 在 `branch/start` 分发时追加的是 Append，不算在内，与 TimelineTree 的创建规则相同）：首行为 `<dim-goal kind="evaluate" branch="B" evaluation="N">` 的用户文本，且 `B` 是本分支 `branch/start` 的 `parent`。

| 消费的事件 | 条件 | 动作 |
| --- | --- | --- |
| `turn/end` M，状态 `completed` | 回复带有结论 | 用 `treeCommands.input` 写到 `B`（Append），标记 `evaluation="N" evaluator="E"` |
| `turn/end` M，状态 `completed` | 没有结论，且 `M` 是评审分支的第一个回合结束 | 在评审分支上写一条追问（Queue，`after="M"`），要求它以三种结论之一结束 |
| `turn/end` M，状态 `completed` | 没有结论，且 `M` 不是第一个回合结束（已经追问过一次） | 向 `B` 写 `continue`，以截断到 4000 字符的整个回复为反馈 |
| `turn/end` M，状态 `failed` 或 `cancelled` | — | 向 `B` 写 `failed` |

向 `B` 的写入返回后，Goal 在 `E` 上写报告标记（Append，无 key，标记 `after="M"`，§5），使 `M` 在 `E` 自己的折叠中成为 `decided`，与追问的标记方式相同。

回复取该回合最后一次模型请求：按序号顺序拼接 `turn_id` 匹配、且 `request_id` 等于该回合最后一个 `step/model-start` 的每个 `step/text` 的 `delta`（一行可能跨多条记录；更早的请求和思考不算）。结论从最后一个以 `DONE:`、`CONTINUE:` 或 `BLOCKED:` 开头的行开始，到末尾为止，所以 `CONTINUE` 清单可以跨多行；截断到 4000 字符，作为 `continue` 写回的整个回复也一样。

是否已经追问过，由评审分支自己的历史决定：以 `M` 结束的是它的第一个回合还是之后的回合。重载时和 Goal 一样，对尚未决定的最新 `turn/end` 做一次决定，不重复前一代已提交的写入（[分支 TypeScript 绑定](../../../design/branch-typescript.md#subscription-rebuild-and-recovery) 第 2 条）：重启截断的评审回合以 `cancelled` 结束（终态事实），于是写出 `failed`；报告标记已在 `E` 上的 `turn/end` 是 `decided`，不再写任何东西。唯一的重复发生在向 `B` 的写入提交之后、标记写入之前停止：重载后的评审者再写一次同样的报告，工作分支只对 `N` 的第一份结论做决定，干活的模型会再看到一次这份报告。标记放在写入之后，因为相反的顺序在两者之间停止时会丢失报告，使目标保持 active 却没有评审在进行。

## 5. 上下文映射

标记、转义和 key 沿用 Goal（[A.5](Goal.md#a5-模型可见文本)）。`N` 是请求所在回合的 `turn/end`，`B` 是工作分支，`E` 是评审分支。模型可见文本保持英文。

| 写入 | 位置、key | 模式 | 文本 |
| --- | --- | --- | --- |
| 评审提示词 | E，第一条输入 | Queue | `<dim-goal kind="evaluate" branch="B" evaluation="N">` / `You are an evaluator. Branch B reported that its goal is completed. Only you decide whether it is. Do not continue the work, do not change files or settings, and do not call goal_update.` / 空行 / `Objective:` / `<objective>` / 空行 / `Reported result:` / `<summary>` / 空行 / `Verify the facts yourself by reading files and running checks; do not rely on the report or on earlier messages. End your reply with exactly one of:` / `DONE: <the verified result>` / `CONTINUE: followed by the items that are at least still missing, one per line, each with its evidence` / `BLOCKED: <what only the user can provide>` |
| 报告标记 | E，无 key | Append | `<dim-goal kind="reported" result="done\|continue\|blocked\|failed" after="M" evaluation="N" branch="E">` / `The result of this evaluation was written to branch B.` |
| 追问 | E，无 key | Queue | `<dim-goal kind="evaluate-again" after="M" branch="E">` / `Your reply did not end with a conclusion. End your reply with exactly one of DONE:, CONTINUE: or BLOCKED: as described above.` |
| 请求 | B，`goal:status` | Append | `<dim-goal kind="status" status="active" judge="evaluator" after="R" branch="B">` / `You reported that the goal for branch B is completed. An evaluator checks that report when this turn ends; until it reports, the goal stays active. Do not call goal_update again for the same result.` |
| 结论 | B，无 key | Append | `<dim-goal kind="report" result="done\|continue\|blocked\|failed" evaluation="N" evaluator="E" branch="B">` / `Evaluator report:` / `<结论，或 "the evaluator's turn did not complete">` |
| `done` | B，`goal:status` | Append | `<dim-goal kind="goal" op="completed" by="evaluator" after="C" evaluation="N" branch="B">` / `Result:` / `<result>` / 空行 / `An evaluator verified that the goal for branch B is completed. Do not continue working toward it on your own.` |
| `blocked` | B，`goal:status` | Append | `op="blocked" by="evaluator" after="C"`；`Blocker:` / `<what is needed>` / 空行 / `An evaluator found that the goal for branch B cannot proceed without the user. Do not continue working toward it on your own.` |
| `continue` | B，Goal 续跑 | Queue | `<dim-goal kind="continuation" count="k" limit="L" after="C" evaluation="N" branch="B">` / `An evaluator checked your report that the goal for branch B is completed and found it is not achieved yet. Its report above lists items that are at least still missing; the list may be incomplete. Assess yourself how far the goal still is, then continue working toward it. When you think it is achieved, call goal_update with status "completed" again.` |
| `failed` | B，`goal:status` | Append | Goal 的暂停文本，原因短语 "the evaluator could not check the result"，标记 `after="C"`、`after="F"` 或 `after="N"`（§4.1） |

请求块取代干活模型的 `completed` 块成为最新的状态块，所以重放出的状态始终与折叠一致，没有例外（Goal A16）：从干活模型的块到请求块之间是 `completed`，之后是 `active`，直到结局或变更取代它。它的标记是客户端已经按 active 读取的状态块（[Goal A.3](Goal.md#a3-读取目标)）。结论输入和报告标记与 Goal 自己的写入一样，没有 key，也没有 `user` 内容，所以不改变状态，不是操作者输入，也不重置计数（Goal A5）。

## 6. 规则检查

规则：[原则 §1 与 §4](../../../architecture/rust-agent/principles.md)、[ADR 002](../../adr/002-plugin-scope.md) D1–D5。

| 规则 | 本文档如何遵守 |
| --- | --- |
| §4.1 用指令事件改别人的状态 | 创建评审分支是发给 TimelineTree 的指令；结论是写给工作分支的输入，由那里的 Goal（所有者）决定 |
| §4.2 交接后不撤回 | 不撤回任何东西；在标记之前停止时重复的报告只是不做任何决定 |
| §4.3 只有 before 回调，不阻塞 | 不适用：没有 before 回调 |
| §4.4 核心不带业务含义 | 不改 Timeline、Dispatcher、TimelineTree、InputQueue 或 Loop |
| §4.5 批次 | 决定按已消费事件的序号顺序做出；每个结局是一条输入 |
| §4.6 上下文只含交接后的事件 | 干活的模型只通过已采用的输入看到结论；评审者的回复留在它的分支上 |
| §4.7 只用公开事件和服务 | Goal 只读本分支的公开事件，调用已导出的服务；没有分支读取另一个分支 |
| §4.8 向机制所有者提需求 | 无 |
| §4.9 只有后续决定要用的事实才进 Timeline | 不新增事件类型；`branch/create` 记录评审，请求块是折叠和模型读取的状态，结论输入是所有者据以决定的事实，追问是让评审者给出结论的事实，报告标记和结局的 `after=` 标记使重载既不重复也不丢失决定 |
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

1. 评审裁判：续跑是 Goal 的。干活的模型报告 `completed`；Goal 写入请求块，所以最新的状态块读作 `active`；该回合结束时在 `N` 处创建一个带 `goal:evaluate:N` 的评审分支，不写续跑。`DONE` 得到评审者判定的 `completed`。
2. 带两行清单的 `CONTINUE` 得到结论和一次续跑（在 `max(A, last_operator)` 之后的续跑上 `count` + 1；请求块和结论都不移动 `A`，也不重置计数）；有待处理的用户输入时 `CONTINUE` 不写续跑，该输入的回合结束续跑一次；`BLOCKED` 得到 `blocked`；达到 `maxContinuations` 时 `CONTINUE` 得到暂停（`limit`）。
3. 干活模型的 `blocked`、上限暂停、用户中断和用户暂停都不创建评审分支。
4. 没有结论的评审回复在评审分支上得到一次追问，之后的结论生效；如果第二次回复仍没有结论，截断到 4000 字符的整个回复作为 `continue` 写回（受 `maxContinuations` 约束的一次续跑），不再追问，重载后也一样。
5. 评审回合失败或取消、`branch/create-failed`，以及在任何 `branch/create` 之前抛出的 `create`，得到一次暂停（`evaluation_failed`）。
6. 评审期间重启：没有第二个评审分支；被截断的评审回合得到 `failed`；报告标记已写入的评审分支重启后不写第二份报告，在标记之前停止而重复的报告只生效一次；结论提交之后、结局写入之前重启的工作分支写出一次结局，结局之后重启的不写任何东西。
7. 来自非记录评审分支的结论不做任何决定。
8. 流式输出的结论能跨 `step/text` 记录重组，并忽略更早请求中的 `DONE:`。
9. 模型裁判：Goal 的验收不变。
10. 在工作分支上的 `set`、`edit`、`clear`、`pause`、`resume` 或干活模型新的 `completed` 之后到达的结论不做任何决定：`pause` 之后迟到的 `DONE` 让目标保持暂停。
11. 折叠没有例外（Goal A16）：请求块之前，最新的 `goal:status` 块和折叠都读作 `completed`，之后都读作 `active`；在请求块之前被消费的回合结束，在请求块被消费后得到决定。

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
| 评审裁判下以请求块回应干活模型的 `completed` | 只有评审者能使目标完成，而最新的状态块仍然就是状态（Goal A16） |
| 在停下的 `turn/end` 处创建 | 这是触发点；`branch/create` 也让重载不会重复创建 |
| Goal 分支部分中的评审角色 | 评审分支必须报告结论，而只有那个分支上的部件能读到它的回复 |
| 写给工作分支的结论输入 | 唯一的跨分支途径；由那里的 Goal 决定 |
| 只取来自记录评审分支、且 `N` 仍为当前评审时的第一份结论 | 报告标记之前停止会写两次，操作者可能创建冒充的分支，迟到的结论也不能覆盖更新的目标变更 |
| 评审分支上的报告标记 | 没有它，每次恢复都会向工作分支再写一份模型可见的报告 |
| 结局的 `after=` 与装载时的决定 | 在结论或创建失败与其结局之间重载，会让目标保持 active 却没有评审在进行 |
| `continue` 上的 C5 | 评审者工作期间用户回合写出的续跑之上不能再叠加一次续跑 |
| 回复没有结论时追问一次，之后按 `continue` 处理 | 否则评审会在没有结果的情况下结束，或无限追问 |
| 评审回合失败或取消、创建失败时写 `failed` | 这些是终态事实；没有它，目标会保持 active 却没有评审在进行 |

相对上一版删除了：声明和重新打开状态、截止时间与 `evaluationTimeoutSeconds`、`goal/evaluation-reported` 记录、评审期间针对用户输入的特殊规则，以及停止和退役分支的需求。

## 12. 进入 Ready 还差什么

没有待定的产品决策；本设计等待维护者确认。#252 的评审跟进修改了 P4，使 Goal A16 不留例外（用请求块代替把干活模型的 `completed` 读作 `active`），并增加了报告标记、结局的 `after=` 标记及其装载时的决定、两条失败路径只写一次暂停，以及 `continue` 上的 C5；作者连同其余部分一起确认。
