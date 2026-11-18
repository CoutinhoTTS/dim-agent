# Timeline 分支

状态：已采纳（[#427](https://github.com/arcships/dim/issues/427)）。本文同时是需求与设计：§1–§3 回答 #427 的问题，§4 起是设计。范围：TimelineTree 插件（TimelineTree、TreeTimeline、BranchTool，BranchTool 新增一个分支部件）、两个 App Server 内置方法、三种核心事件、CLI TUI，以及[设计规则 §2](../../architecture/rust-agent/principles.md#2-作用域的含义) 的改写。语言：Rust。English: [English](../../design/timeline-branches.md)。

## 1. 分支用来做什么

**分支**是同一个 agent 在一个 Session 里的一条行为线。Session 是一个主体，也就是"我"：它的身份、它与用户的交互面、它的能力和它的工作区。分支是这个主体的一条行为线。

子分支从父分支的历史中某个切点开始。切点之前的每个事件也是子分支自己的历史，这些行不会改变。切点之后，子分支读自己的事件，并通过 `branch` 工具读它被切出来的那条线（§6）。

分支之间共享事实，不共享上下文：

- 每条线用自己的事实组装自己的上下文。
- 每条线各自压缩。
- 一条线影响另一条线，向那条线的 Timeline 写输入；它了解另一条线，靠自己对那条线的读取。
- 祖先追加了投递输入、但该输入还没有在继承前缀里被消费的规则或状态变更，不属于这条线继承的状态：本线按来源的当前值自行采用，那个块是本线切点之后自己的事件（[prompt](../../architecture/rust-agent/prompt.md)）。

父子关系记录历史的来源，也是分支之间唯一可以通信和读取的关系。它不表示控制，也不表示欠对方一个结果。

对 #427 的回答：

| # | 问题 | 回答 |
| --- | --- | --- |
| 1 | 支持哪些场景，按什么顺序 | 三个，依次是（§3）：插件开的线（已交付）、agent 从主线开的线（本设计）、用户从历史某点开线、再比较和采纳（以后） |
| 2 | 分支、Collaboration 成员、fork 怎么区分 | 见 §2。分支是同一个 agent；成员和 fork 是另一个 Session |
| 3 | 分支结束时，谁需要它的结果，以什么形式 | 没有哪条线欠另一条线结果。各线在工作过程中把要紧的事告诉对方（§6）。agent 开的线结束时给开它的那条线一条最终消息（§10）。插件开的线按创建者的协议办，例如 Pursuit 的结论 |
| 4 | 分支什么时候算结束，结束后还能看到吗 | 它自己结束、它上面的线结束它，或者用户结束它（§10）。结束的线留在分支目录里，客户端和开它的那条线仍能读到它的历史 |
| 5 | 用户能看到什么、做什么 | 跟随任一条线、与它对话、回答它的提问和审批、结束它（§9、§14）。比较和采纳随场景 3 一起做 |
| 6 | 怎样算"完成" | 验收流程（§20）通过，评估场景（§12.3）已在本地用真实模型跑过，并在实现 PR 里报告。发布演示（[#439](https://github.com/arcships/dim/issues/439)）可以展示它 |

## 2. 分支、Collaboration 成员、fork

| | 分支 | Collaboration 成员（subagent） | Fork |
| --- | --- | --- | --- |
| 是什么 | 同一个 agent 的另一条线，在同一个 Session 里 | 另一个有自己档案的 agent，在它自己的 Session 里 | 从 `main` 复制出来的一个新的、独立的 Session |
| 历史 | 父分支切点之前的历史，之后是它自己的 | 从任务开始，没有 leader 的历史 | 复制 `main` 到某个静止点为止的历史（[fork](appserver/session-deletion-and-fork.md)） |
| 身份与能力 | Session 的：同一套 system、工具、模型、权限和工作区 | 它自己的档案：指令、工具和模型 | 配置的副本，之后各自独立 |
| 关系 | 父子在工作过程中互相写输入。没有任务约定，也没有控制 | leader 给它任务，结果自动返回。成员不能再委派 | 复制之后没有关系 |
| 何时用 | 同一个 agent 在同一批事实上并行工作 | 工作需要另一个身份、另一份档案，或单独的上下文 | 要从过去某点在别处继续，而不动这个 Session |

设计规则 §2 的规则不变：工作需要不同的身份或能力时，建一个新的 Session（成员或 fork），绝不开分支。

## 3. 场景

| # | 场景 | 状态 |
| --- | --- | --- |
| S1 | **插件开的线。** 插件按自己的设计开线。例如 [Pursuit](plugins/Pursuit.md) 的评审者：它是一条插件线，切在执行者那一轮结束处，结论以输入的形式写回（已实现，[#422](https://github.com/arcships/dim/issues/422)；`crates/dim-goal`） | 已交付。本设计给它加上身份块（§7.1）。Goal 对评审者的识别和结束随之改变（[Pursuit §13](plugins/Pursuit.md#13-结束评审者随-427)） |
| S2 | **agent 从主线开线。** 例如：主线继续读代码时，在两条线上分别验证一个 bug 的两种假设；主线实现的同时，让一条线随改动逐个评审；主线继续工作时，让一条线调研某个 API 或代码的某个区域；在另外的文件上尝试第二种做法 | 本设计 |
| S3 | **用户从历史某点开线、比较、采纳。** `session.branch_create` 已经能开线。有了本设计，用户可以跟随这些线、与它们对话、把主线移到其中一条、结束它们。比较和采纳需要单独设计：采纳一条线，对主线的上下文和共享的工作区意味着什么 | 以后，在本设计之后 |

下面这些需求不是分支：

| 需求 | 为什么不是分支 | 归属 |
| --- | --- | --- |
| 委派给有自己指令的专门角色 | 那是另一个身份 | Collaboration 成员 |
| 判断是否批准一次工具调用 | 那是对一个动作的决定，不是一条工作线 | 带宿主端口的能力（[ADR 003](../adr/003-approval-scope.md)） |
| 把对话回退到更早的某点 | 它改变的是这条线如何看自己的历史，不是一条新线 | 单独的需求。目前的答案是 `session.fork` |
| 隔离地并行编辑 | 各线共用 Session 的工作区，没有按线隔离的工作区；两条线因此不能同时写同一批文件 | fork，或有自己工作目录的成员 |
| 跑一条长命令，或等会话之外的事情 | 一条命令是后台工作，外部事件是当前线可以等的一个信号；两者都不需要第二条推理线 | 后台命令，或事件订阅 |
| 把一个大目标坚持做到底，无论多少轮 | 那是这条线的继续，不是它旁边的第二条线 | 这条线，或 goal |

## 4. 树与角色

- **深度。** 架构上分支可以有任意深度。`session.branch_create` 接受任意父分支；Goal 接受任意 `branchId`，所以开在子分支下的评审者是孙分支。
- **亲缘。** 父子是分支之间唯一可以通信的关系。父分支和它的每个子分支互相写输入，一条线还会读它被切出来的那条线（§6）。兄弟分支、更远的亲属之间既不发消息也不读。这条规则约束的是一条线作用于另一条线，在这里就是 `branch` 工具。能力按自己的决定行事，属于能力影响分支，不受这条规则限制（[设计规则 §2](../../architecture/rust-agent/principles.md#2-作用域的含义)）。例如 Goal 写入评审结果，或插件在任意切点开线。
- **根。** `main` 是没有父分支、也没有来源的那条线：这个字面量 id、它的编号，以及所有已存下的 `branch_id: "main"`，本设计都不改。它永不结束，Session 关闭不算结束。界面上它叫“起点”。
- **主线。** 这个 Session 的对话所在的那条线：客户端不传 `branchId` 时采用的目标，也是客户端居中显示的那条线。它是一个角色，不是 id：开局就是根，只在用户动作下移动（§9、§14）。唯一权威的事实不是模型可见的块，而是写在根线上的一条 `branch/primary {branch}`（根线比其它线都活得久，且不能结束）（§16）。告诉各条线这件事的键控块从来不是权威。没有这条事实的 Session 就在根上。
- **开线方。** 开过线的线就协调这些线：它有自己的名册（§7.1），也只有它能给自己开的线发消息、读它们、结束它们。协调关系沿父子关系走，从不随主线移动。
- **agent 的工具。** `branch` 工具只能从当前主线开线。不是主线的线会被拒绝。只有插件线永远不能成为主线：工具线、用户线和根都是合法目标（§14；根在它不是主线时即可）。树的深度只在用户把主线移到一条会自己开线的线时才增长。一个 Session 里工具开着的线总数有上限（§6.1）。
- **来源。** 每条线都有来源，记在父分支的 `branch/create` 里：
  - **工具线**由 `branch` 工具开出。它是对话线：可以成为主线，当了主线就能开线。
  - **用户线**由客户端经 `session.branch_create` 开出。它的开线方是用户，它同样是对话线：可以成为主线，当了主线就能开线。
  - **插件线**由插件（`TreeCommands.create`）开出。它的协议归创建者，既不能成为主线，也不能开线。

  来源可以从 `TimelineInfo.source` 读到，由 `TreeCommands.list` 和 `info` 返回，根上没有（§5）。插件线的来源是 `{kind: "host", operation_id}`：开线一方的不透明标签，核心从不解释它。用户线有它自己的种类 `{kind: "user", operation_id}`，所以需要分辨对话线与插件线的读者不用去解析 `operation_id`。开线一方按自己的 operation id 约定找到自己的线，例如 Goal 的 `goal:evaluate:` 前缀。
- **工具的作用范围。** 工具的 `read` 读调用方被切出来的那条线，也读它开过的线；`message` 和 `end` 也在这两对之间生效。插件线不在任何名册上，工具不给它发消息，也不结束它。
- **父线块。** 一条线的 BranchLine 在父线结束一轮、且父线在 `max(读取游标, 上次告知的头部, 开启点)` 之后有新活动时，在这条线自己身上写一个带 key 的 `<dim-branch-parent>` 块（§7.1.1、§12.1）。块只报位置，是状态而不是请求；内容仍留在读取返回的页里。

| 线 | 用工具开线 | 用工具发消息、读 | 由谁结束（按约定） |
| --- | --- | --- | --- |
| `main`，根 | 当它是主线时能 | 它开过的线 | 永不结束；Session 关闭不算结束 |
| 当前主线 | 能 | 它被切出来的那条线，以及它开过的线 | 它在主线期间谁都不能结束它，用户也不行：要结束它，用户先把主线移到另一条线上 |
| 其它对话线（工具线或用户线） | 不能（工具结果拒绝） | 它的父分支以及它开过的线 | 它自己（`end`）、用户，或祖先结束时连带 |
| 插件线 | 不能 | —（按创建者的协议） | 它的创建者（Goal 结束它的评审者，[Pursuit §13](plugins/Pursuit.md#13-结束评审者随-427)）、用户，或祖先结束时连带 |

谁结束哪条线，是调用方之间的约定。运行时只检查目标，不管是谁在请求，用户也一样：它存在、未结束，且既不是根、也不是当前主线、也不在主线的祖先链上，因为结束一条线会连带结束它下面的每一条（§10）。

## 5. 部件

所有部件都属于 TimelineTree 插件（[timeline_tree.rs](../../../crates/dim-agent-capabilities/src/timeline_tree.rs)），每个部件放在拥有其状态的作用域（[ADR 002](../adr/002-plugin-scope.md) D1）。

| 部件 | 作用域 | Capability | 在本设计中负责 |
| --- | --- | --- | --- |
| TimelineTree | Session | 否 | 线的生命周期：每个子分支开启输入里的身份块、`TimelineInfo.source`、主线与其 `branch/primary` 事实、一条线的结束（`branch/end` → `branch/ended`）、线进入结束后拒绝输入、恢复规则、读已结束线的历史，以及分支目录（§14） |
| TreeTimeline | 分支 | 是 | 把 `branch/end` 转给 TimelineTree，就像现在转 `branch/create`。对由工具调用发出的结束，它还转交关闭要等的那一轮结束 |
| BranchTool | Session | 否 | 按动作区分的 `branch` 工具（§11）。它每次调用时取调用方那条线的 BranchLine，与 WorkspaceTools 取 FileReads 的方式相同 |
| **BranchLine**（新） | 分支 | 是 | 一条线对自己相关各线的视图。它的角色（根、主线、开线方、插件线）和父分支来自 `TreeCommands.info(自己的 id)`。它从自己的事实折叠出自己开着的线和当前这一轮的状态。它写自己的名册、空闲通知、结束通知，以及主线移动时给它的那条状态说明 |

BranchLine 注入 `B:Dispatcher`、`B:TimelineIdentity`、`B:BranchLifecycle` 和 `S:TreeCommands`，提供 `B:BranchLine`。

它的决定方式与 Goal 相同：依据已消费的状态，通过一个写入前会复核的自有写任务，并且只对加载截点之后的实时事件作决定。所以重载时什么都不会写两次，只有一个例外：如果加载时自己的历史里最后有一个仍在生效的名册键，而与折叠出的名册不一致，它就把名册写一次——因为补完一条已结束线的重启（§20.7）在开线方重放之前就记下了 `branch/ended`，之后没有实时事件跟随（§6.2）。

**线的来源。** TimelineTree 依据父分支的 `branch/create` 填写 `TimelineInfo.source`：

- 创建时，取自这次创建本身；
- 恢复时，取自恢复中的父分支对自己历史的读取。

恢复中的父分支在构建时把历史读到它的头部，并报告它记录为失败的创建（[timeline_tree.rs](../../../crates/dim-agent-capabilities/src/timeline_tree.rs) 中 `Opening::Resume` 的 `failed_creates`）。它还报告每条 `branch/ended`，恢复规则会用到它们（§10）。存储的目录行不变（[存储格式](../../design/appserver/native-storage-format.md#427-v20-格式决定)）。

**主线的恢复。** 目录里只有一条线的 Session 就在根上，不为它读任何事实。有多条线的 Session 只读一条事实：在根自己的事件里，最新的一条 `branch/primary` 或 `session/forked`，走现有的 `timeline_events_by_kind` 索引。目标仍未结束时，那条 `branch/primary` 指明主线；最新的一条是 `session/forked` 时，主线就是根，因为 fork 拷过来的所有 `branch/primary` 都写在它的 `session/forked` 之前、一律失效；什么也没记就是根。目标已经不再是开着的线时回退到根；一条指向插件线的 `branch/primary` 也回退到根。

## 6. 线之间的消息

消息是接收方 Timeline 上的一条 `input/message`，经 `TreeCommands.input` 写入，不新增输入通道。它带一个原始块，由生产者标记、不带 key，以及 [InputQueue](../../architecture/rust-agent/plugins/InputQueue.md) 的模式：

| 模式 | 对接收方的作用 | 何时用 |
| --- | --- | --- |
| `append`（默认） | 随接收方下一次模型请求一起送达。它从不开启新的一轮，也不唤醒 `wait` | 接收方在下一步应该知道的进展或背景 |
| `steer` | 在接收方当前这一轮的下一步进入；接收方空闲时开启新的一轮。它会唤醒 `wait` | 接收方必须马上处理，或接收方处于空闲 |

不开放 Queue。一条线给另一条线的唯一 Queue 输入是开启输入。

这两个模式和用词与 Collaboration 的消息相同（`append`、`steer`），含义也相同，因为两者用的都是 InputQueue 的模式。

- **标记。** `<dim-branch-message from="C" to="P">`。接收方凭这个标记把消息和用户的话区分开。RPC 只接收操作者内容、不接收块，所以客户端无法伪造消息。
- **允许的两端。** 消息与读取：一条线和它开过的线，双向；以及一条线和它被切出来的那条线，双向。协调沿父子关系走，所以它留在开线的那条线上，不随主线移动而搬家。其他任何组合都由工具结果拒绝，什么也不写：兄弟分支、一条线和它的祖父级、插件线在两个方向上的任何一对、已结束的线。拒绝码仍是 `branch_not_kin` 和 `branch_not_tool_line`。
- **读取。** `read {branch, after?, limit?}` 返回目标已提交历史的一页，未结束或已结束都可以，经 `TreeCommands.page` 从存储读取，按 Collaboration 的 `read` 渲染成员页面的方式呈现：输入、文本、工具调用与结果、每轮结束，带 `nextCursor`；工具结果条目携带其工具在该结果上声明的效果（[工具契约 A.6](contracts/session-boundary.md#a6-工具效果)），仅当结果至少声明了一个时才存在，路径原样，命令在第 200 个字符处截断并标出截断。目标是调用方被切出来的那条线，或调用方开过的某条线。两个方向共用一条游标规则：
  - 不带 `after` 时，读取从调用方上一次成功读取该线的位置继续：最后一次返回了页的读取的 `nextCursor`。此前没有读过时，一条线第一次读父线从它的切点开始，第一次读它开过的线从那条线的第一个事件开始。
  - 显式 `after` 从该点读取。读父线时，低于切点的值会抬到切点，因为切点之前的事件已经是这条线自己的历史。读它自己开的线时，值原样使用，所以 `after: 0` 会从头重读。
  - 每次成功读取都把游标移到它的 `nextCursor`，显式 `after` 的读取也一样；以最后一次为准。游标是调用方自己那条线的事实：BranchLine 从调用方自己的 `step/tool-use` 和 `step/tool-result` 折叠得到，所以重载和压缩都能重建它。
  - 一页可能在目标某一轮的中间结束：它读到目标已发布的头部，下一次读取从这一页之后继续。
- **知道何时读。** 读取是按需的，所以一条线还需要知道父线动了。带 key 的 `<dim-branch-parent>` 块（§4、§7.1.1）给出父线的头部和这条线的游标；线据此决定读哪些页。块从不携带内容，读这个块也不是在读父线。
- **不设上限。** 线的数量、消息数、steer 数都不设上限。消息往复由提示词规则（§12）、某条线自行结束，或用户（中断或结束）来终止。为什么这样是安全的，以及需要时由谁负责限制，见 §6.1。
- **空闲通知。** 一条工具线的一轮结束时，这条线上的 BranchLine 给开它的那条线写一条 steer（`<dim-branch-idle>`，§12.1），除非下列之一成立：
  - 创建时指定了 `idleNotice: none`；
  - 这条线就是当前主线；
  - 这一轮已经给它的开线方发过 steer 消息；
  - 这一轮调用了 `end`；
  - 这条线还有待处理的驱动输入。

  这样，一条线一声不响地停下时，它的开线方不会一直等下去；已经 steer 过开线方的线，也不会重复通知它。通知带这一轮的状态（`completed`、`failed` 或 `cancelled`）。
- **等待。** 一条线用现有的 `wait` 工具等它开过的线。steer 消息、空闲通知或结束通知都会唤醒它（[wait.rs](../../../crates/dim-agent-capabilities/src/wait.rs)）。

### 6.1 资源与限制

不对消息设上限是安全的，因为线没有带来新的资源类别：

- 一条线的工作就是同一个 Session 的普通轮次，使用同一个 provider，受同样的 Permission 和 Approval 约束。
- 一条线同一时间最多运行一轮：Loop 每条线只持有一轮（[agent_loop.rs:106](../../../crates/dim-agent-capabilities/src/agent_loop.rs)）。
- 一条消息是由一次工具调用写入的一条有界的 `input/message`，大小受模型自己的输出约束。
- RPC 准入按连接设有上限（[架构](../../architecture/README.md)）。
- Collaboration 的成员也没有数量上限。

开着的工具线有上限。这个上限是整个 Session 的总数：`branch` 工具开过、仍然开着的线，在全树范围内统计，不是每条名册各自一份，也不只算根线上的。客户端开的线和插件线从不计入，所以它们也不会被这个上限拒绝。BranchTool 拥有这个限制；达到总数时它用自己的拒绝码拒绝 `create`，于是模型读到的是一个能应对的拒绝，而不是一条开不出来的线。这项检查放在创建时本来就要做的祖先回溯里，在同一把树锁下完成，所以两条线同时创建不会都通过。树深不单独设上限：它只在用户移动主线时增长，而总数已经限住了工作量。数值由测量（§12.3）决定，并在实现 PR 里报告。

如果以后还需要别的限制，每一项都已有归属，而且都不需要改机制：

| 限制 | 归属 |
| --- | --- |
| 开着的工具线数 | BranchTool，按整个 Session 的总数，在创建时于树锁下检查；客户端开的线从不计入 |
| steer 频率 | 发送方的 BranchLine 统计它自己 Timeline 上的 steer 消息 |
| 整个 Session 的模型成本或并发 | 归模型调用层。这不是分支特有的限制：它同样适用于成员和 fork |

其余的保护措施是：

- 提示词规则（§12）；
- 已结束的线拒收消息；
- 用户的结束和中断；
- §12.3 中的刷屏、过度拆分和来回空转场景，在本地测量并在实现 PR 里报告，从不作为 CI 阈值。

开着的线确实有代价：

- 开着的线保留它已加载的部件。
- 重启时每条开着的线都会重新加载。Session 关闭和空闲卸载都不会结束一条线。
- 开着的工具线和用户线会让开它的那条线的名册变长，名册只列它开着的线。每次从线外结束一条这样的线，它的开线方上就多一条通知。

永不结束的线由它自己的协议（`end`）和用户负责，它开线方的名册让它始终可见。Pursuit 的评审者由 Goal 结束（[Pursuit §13](plugins/Pursuit.md#13-结束评审者随-427)）。

### 6.2 已知代价

- **重启可能丢掉一条空闲通知。** 如果进程停在一条工具线一轮结束之后、它的空闲通知写出之前，这条通知就永远不会写出，因为 BranchLine 只对加载之后的实时事件作决定。Pursuit §4.2 也留着同样的窗口。开线方的名册上仍有这条线，可以 `read` 它，用户也能看到它处于空闲。
- **重启可能丢掉结束通知。** `branch/ended` 与名册输入之间同样有窗口，一条线被外部结束时本该写出的结束通知会丢掉。加载时的补写只写名册，所以开线方是从名册的退场得知，而不是从通知。
- **压缩后的名册很短。** 一条线压缩之后，它的名册只保留每条线的 id 和目的的第一行。开线方告诉过一条线什么，只留在压缩摘要里，或只能经 `read` 看到；而 `read` 会把那条线的原始事件一页一页带进开线方的上下文。
- **变成主线的对话线保留自己的历史。** 把主线移到 agent 开的一条线上，只改变它此后被告知的东西（§9），不改变它已经记下的东西。它旧的身份块和目的仍留在它的历史里，在说明它现在是主线的那条状态说明之上。

## 7. 身份与上下文

### 7.1 带 key 的状态：`branch:line`、`branch:roster` 与 `branch:parent`

每条线用同一个 key `branch:line` 下的一个块，回答“我是哪条线”。开过线的线另用自己的 key `branch:roster` 下的一个块回答“哪些线是我的”。两个 key 分开，是因为现在每条线都可能同时带着两者：一条不是根的开线方的线，既有身份块，也有名册；而共用一个 key 会让写名册把这条线自己的身份退掉。一条线还为自己被切出来的那条线保留一个带 key 的块 `branch:parent`（§7.1.1）。

- **子分支上：它的身份。** TimelineTree 在每个子分支上写入，工具线、用户线、插件线都是。它是子分支开启 Queue 输入的第一个块，key 为 `branch:line`，写明：
  - 子分支的 id、父分支和切点；
  - 它是同一个 agent，上面的历史就是它自己的；
  - 从这里起它并行工作；
  - 上面那些发给父分支的消息（例如父分支的名册）属于父分支。

  工具线的身份块还带上它的目的（`create` 的 prompt）和线协议。在用户线或插件线上，开线方的 prompt 仍是开启输入的 `user` 内容，排在块之后。
- **开线方上：它的名册。** 开过线的线上的 BranchLine，依据这条线自己的事实写一条带 key 的 Append，key 为 `branch:roster`：
  - 工具线或用户线的 `branch/create`；
  - `branch/create-failed`；
  - `branch/ended`。

  名册列出每条未结束的工具线，带它的 id 和目的的第一行。没有未结束的工具线时，这个 key 退役。
- **覆盖。** 子分支在继承的前缀里也带着父分支的名册。它自己同一 key 下的身份块覆盖了那份名册。从子分支的开启输入起，子分支上最新的有效 `branch:line` 块就是它的身份。

key 与模式互不相关（[history.rs](../../../crates/dim-agent-capabilities/src/history.rs) 的 `render_input`）。Goal 的 `set` 已经在 Queue 输入里放带 key 的块（[Goal §5](plugins/Goal.md#5-上下文映射)）。

身份放在开启输入里，而不是在 `branch/start` 时写一条 Append，原因有两个：

- Append 可能在开启输入之后才提交（[Goal §3.4](plugins/Goal.md#34-新分支清除继承的目标)）。
- 在 `branch/start` 时还不知道目的。

### 7.1.1 父线块 `branch:parent`

读取是按需的，所以一条线还需要知道它被切出来的那条线动了。这条线上的 BranchLine 观察父线的轮次结束，在自己 Timeline 上写一条带 key 的 Append，`branch:parent`。这是这条线自己的事实，从不属于父线：没有哪个部件折叠另一条线的事件。

- **信号。** TimelineTree 本来就分派每条线的 `turn/end`；线自己的节点发布它最新一次轮次结束的序号。子分支的 BranchLine 等待父线的信号和自己的写队列，任一变化排入一次决定。决定取决于这条线自己的状态，而不是信号：只要它没有持有 `branch:parent` 块、且父线的头部超过了最新块告知的头部，它就写一个。在块被持有时到达的移动，会在该块被消费、删除或中断时写出，所以不会丢失。没有部件为此读取父线的历史。
- **条件。** 只有以下条件全部成立时才写这个块：
  1. 这条线的角色是工具线或 host 线，且这条线不在结束；
  2. 这条线上没有更早的 `branch:parent` 输入仍被持有，即已提交、尚未消费；
  3. 父线在 `from = max(A, told, opened)` 之后有新活动：`A` 是这条线对父线的读取游标（还没读过时是它的切点），`told` 是这条线上最新一个 `branch:parent` 块报告的头部（没有时为 0），`opened` 是这条线的 `branch/create` 事件在父线上的序号，每个切点都不高于它。线存在之前发生的活动不算新闻，需要时线自己去读。
- **什么算新活动。** 只有两类事件算：操作者输入，即带 `user` 内容的 `input/message`；以及工具调用。创建调用的 `step/tool-use` 写在 `opened` 之前，所以从不会被算进去。其他都不单独算：父线的文本、它的轮次结束、只有块的输入（名册、来自各线的消息）、以及创建调用的结果。规则是：父线行动时、或操作者告诉父线事情时，线被告知；只为开启它的那一轮（它的创建调用结果、文本和轮次结束）则从不告知，否则每条线都会白白读一次父线，并学会忽略这个块。
- **检查方式。** 父线的事实通过唯一的事件查询读取（[ADR 019](../../adr/019-event-query.md)）：该分支的 `input/message` 和 `step/tool-use` 事件，从新到旧分页，直到有一个符合条件，或读取到达 `from`。不折叠任何其他线的事件。
- **写入。** 一条带 key 的 Append，key 为 `branch:parent`，经 `TreeCommands.input` 写在这条线自己身上，属性是父线的 id、`head` 和 `after`。同一 key 的后一个块会在带 key 的基线里取代前一个。写完之后折叠记下新头部和新输入，所以队列里的第二次决定不会再写。
- **开启时。** 子分支继承的前缀里持有活的 `branch:parent` 块时，子分支的开启输入在身份块之后再带一个原始块：key 为 `branch:parent`，活的、不 retire，名字是开出它的那条线 P，`head` 和 `after` 都等于切点 H。它像身份块取代继承的名册那样取代继承的块（§7.1），所以这条线最新的 `branch:parent` 总是名字它自己的父线，它的 `told` 从 H 开始，而 H 等于 `opened`；压缩重放的是这个块，从不重放上面那个，P 移动时照常的新块取代它。只有嵌套的线才会拿到它：从 `main` 切出来的线不继承任何 `branch:parent` 块，它的开启输入逐字节不变。TimelineTree 在写身份块的地方组合它，从子分支继承的前缀的带 key 状态判断这个 key 是否活着，绝不看渲染后的历史。它在开启子分支时读那份状态，如同读一条恢复的分支的 facts；读取失败时这次创建和任何其他开启失败一样失败（branch/create-failed）。
- **随后发生什么。** 运行中的线在下一次模型请求时拿到这个块。空闲的线不会被唤醒：这个块在它下一轮的第一个请求里、先于开启那一轮的输入被消费，这是有文档记录的 Append 规则（[session access](contracts/session-access.md)、[Loop](../../architecture/rust-agent/plugins/Loop.md)）。重载不写任何东西；重启最多写当时已经该写的那个块。根拿不到块：它上面没有东西。
- **记录。** 这条线发送的是它自己配置里持有的 System。指南在每条线上都一样（§7.3），所以 System 没有相对继承来的那份发生移动的线，引用的是记录它的那份配置，无论记录在哪条线的链上；线的 System 移动时，它像任何分支一样写出自己的 `context/configured`。
- **被否决的替代方案。**
  - 在线自己一轮结束时写，错过了这个变化存在的场景：线结束一轮，父线继续，随后用户或父线来找这条线。那一轮不携带块。
  - 在线一轮开始时或它的请求之间写，是在请求准备好之后才响应事件；一条已提交的 Append 可能落在它本该服务的那次请求之后，而且需要 Loop 里的钩子。
  - 根给每条线推摘要或 steer，重复了线自己可以读到的内容，还会唤醒空闲的线。
  - 新的事件类型或存储格式改动没有必要：一条原始的带 key 输入已经承载这份状态。

  这个块只说明位置，不携带内容；内容留在读取返回的页里。

### 7.2 压缩

压缩按线进行。一次压缩是压缩方自己 Timeline 上的一条 `context/compaction` 事实。

- 子分支继承祖先在切点之前的压缩记录。事件 id 在继承的前缀里保持不变，所以 `retain_from` 在子分支上的重放方式不变（[ContextCompaction](plugins/ContextCompaction.md)）。切点之后，子分支各自压缩。
- 子分支的压缩不改变父分支，父分支的压缩也不改变子分支。

带 key 的基线会重放 `retain_from` 之前每个 key 最新的有效块。因此：

- 开线方压缩时，它的名册会重放；
- 子分支压缩时，它的身份会重放；
- 一条线压缩时，它最新的 `branch:parent` 块会重放，所以这个块能活过这条线自己的压缩；最新的状态说明也能。

压缩不删除任何行：这条线对读取游标和最新块的折叠，在重放时从同样的事件重建。

两个 key 让每条线的基线里对“我是哪条线”恰好只有一个答案，对“哪些线是我的”至多一个。继承来的答案不会比这条线自己的答案活得更久，名册也不会把身份退掉。消息不带 key：压缩之后，它们像其他输入一样留在摘要或保留的轮次里。

### 7.3 缓存前缀

各线只在尾部不同：

- 根线确定 system 和模型，主线所在的线也确定它自己从切点继承的那份配置（[prompt](../../architecture/rust-agent/prompt.md)）。
- 提示词来源按根的作用域加载，由子分支继承（[PromptSources](../../architecture/rust-agent/plugins/PromptSources.md)）。
- 每个 Session 只有一个 ToolRegistry，所以每条线的工具列表都一样。这就是不是主线的线上 `create` 由工具结果拒绝、而不是拿掉工具的原因。
- 协议只缓存显式标记内容时，请求会给这份前缀打上缓存断点：最多两个，System 末尾一个，最后一条用户或工具结果消息的最后一个块一个，且只在各自条件成立时才放置（[Anthropic 提示缓存断点](../../../crates/dim-agent-aimux/README.md#anthropic-提示缓存断点)）。
- 身份块是子分支自己的第一个事件，紧跟在继承的前缀之后。所以子分支的第一次请求能复用父分支到切点为止的前缀。
- 名册更新是追加在开线方历史末尾的 Append。
- 父线块，以及主线移动时写的状态说明，都是追加在这条线自己历史末尾的 Append，所以它们从不移动任何线继承的前缀。把主线移到一条已经开着的线上，因此不改变那条线的缓存前缀：只有它的尾部变长。
- 指南在每条线上都一样，包括父线块的条目。

### 7.4 Goal 与 Pursuit

**规则。** 一条线的开启输入由 TimelineTree 写入。它的第一个块是 `branch:line` 标记，所以按 [Goal A5](plugins/Goal.md#附录-e-假设)（操作者输入是"带 `user` 内容、且第一个原始块不是来源标记"），它是有来源的输入，不是操作者输入，不管创建者给了什么 `user` 内容。Goal 的冲突表记录了这一点（[Goal 附录 B](plugins/Goal.md#附录-b-完整冲突表)）。Goal 自己的规则不变。它的代码只随 [Pursuit 修订](plugins/Pursuit.md#13-结束评审者随-427)改变。

在本设计之前，插件线的开启输入（创建者的 prompt 作为 `user`，没有块）算操作者输入。工具线的不算：它带 `dim-branch-task` 块，没有 `user`。现在两者都是有来源的输入。

为什么这不改变 Goal 的任何行为：

| Goal 的决定 | 身份块的影响 |
| --- | --- |
| 基于块的决定：已决定集合、续跑列表、`conclusionOf`，以及带 key 的目标和状态折叠 | 没有影响。`marker()` 只匹配 `^<dim-goal `，折叠还要求 `goal:*` key，所以这些决定都看不到身份块 |
| 操作者判断 | 它唯一改变的字段：开启输入不再设置 `lastOperator` |
| `count` | `lastOperator` 只在这里使用：`since = max(active, lastOperator)` |
| 计数窗口 | 折叠只看本线的事件，并在这条线自己的开启处从头开始。子分支开始时没有目标，Goal 会写入开始时的 `clear`（[Goal §3.4](plugins/Goal.md#34-新分支清除继承的目标)）。`active` 只在之后的本地 `set` 或 `resume` 时移动，而这两者总在开启输入之后。所以开启输入从不改变 `since`，计数保持不变 |
| 评审者识别 | [Pursuit 修订](plugins/Pursuit.md#13-结束评审者随-427)之后，Goal 从 `TimelineInfo.source` 和 `parent` 识别评审者，不再读第一条 Queue 输入的 `user` 文本。身份放在 `blocks` 里，`render_input` 先渲染原始块，再渲染操作者的话（[history.rs:181](../../../crates/dim-agent-capabilities/src/history.rs)），所以模型先看到身份，再看到 prompt。上面的 A5 规则照原文不变 |

## 8. 工作区

各线共用 Session 的工作区。没有哪条线能知道文件是谁改的，本设计也不去归属改动。现有的保护已经够用：

- FileReads 按线保留读取绑定。
- 文件在这条线上次读取之后改变时，`edit` 和 `write` 以 `file_modified_since_read` 拒绝，不管是谁改的（[edit.rs](../../../crates/dim-agent-capabilities/src/workspace_tools/files/edit.rs)、[write.rs](../../../crates/dim-agent-capabilities/src/workspace_tools/files/write.rs)）。

其余靠协议，写在线的身份块和工具描述里：

- 开线方在 `create` 的 prompt 里给它开的每条线分配自己的文件或区域。
- 一条线要改它区域之外的文件之前，先告诉它的开线方。
- 遇到 `file_modified_since_read` 时，这条线重新读取文件，告诉它的开线方之后再继续。把主线移到自己的那条线，第一次改上一条主线改过的文件时可能遇到这个失败：这是守卫在正常工作，状态说明里写明了。

文件改动只有工具声明的一种事实，即 `fileWritten`：`edit` 和 `write` 只要已经发布了文件，就在结果的 `effects` 中声明它和路径——成功时如此，发布之后失败（例如同步父目录失败）时也如此；在发布之前失败或被取消的调用不声明（[工具契约 A.6](contracts/session-boundary.md#a6-工具效果)）。客户端折叠这些声明得到的集合在构造上就是部分的——它漏掉 `exec`、MCP 工具以及 Dim 之外对文件的改动——所以折叠描述的是工具声明的写入，而不是工作区的改动。没有任何方法承诺一轮或一条线的完整改动集。

WorkspaceTools 不变。

## 9. 用户

- 交互面属于 Session。每条线都以这个 Session 的身份与用户打交道。
- **提问。** 任何一条线都可以用 `ask_user` 提问。答案作为这次调用的工具结果，回到提问的那条线（[Questions](plugins/Questions.md)）。
- **审批。** 一条线工具调用的审批直接交给用户。审批通知带 `branchId`，客户端会标出是哪条线。
- **不经主线转达。** 这是同一个主体在问它自己的用户。
- **提交。** 用户可以向任一条未结束的线提交（带 `branchId` 的 `session.submit`）。不传 `branchId` 的调用送给当前主线。
- **中断。** 中断一条线只结束它当前这一轮，不结束这条线。不传 `branchId` 的停止停的是当前主线。
- **结束。** 除根、当前主线、以及主线的每一条祖先以外，用户可以结束任一条线（`session.branch_end`，§10）。当前主线谁也不能结束，用户也一样：要结束它，先把主线移到另一条线上。
- **移动主线。** 用户可以把主线移到任意一条开着的对话线（工具线或用户线）上，也可以移回根，用一个方法（`session.branch_primary`，§14）。根只要不是当前主线就是合法目标；插件线永远不是。旧主线照旧开着，并保留它开过的线；什么也不拷贝，历史也不移动。移动在根上写下事实，并只给两条线状态说明：新主线和旧主线（§12.1）。之后旧主线不再能开线，新主线可以。
- **移动不做什么。** 它改变的是这个 Session 的对话在哪条线上。它不碰工作区，所以上一条主线改过的文件保持已改；它不移动 `/fork`，fork 仍然只把根拷到某个停顿点（[fork](appserver/session-deletion-and-fork.md)）；它也不结束任何一条线。

## 10. 结束一条线

| 事件 | Timeline | 数据 | 角色 |
| --- | --- | --- | --- |
| `branch/end` | 要结束的那条线 | `{source, ancestor?}` | 指令 |
| `branch/ended` | 它的父分支 | `{branch, endEvent, source, ancestor?}` | 结果，由 TimelineTree 写入 |

这一对事件与 `branch/create`、`branch/create-failed` 对称：一条 Timeline 上的指令由 TreeTimeline 转交，结果由树写出。`endEvent` 是 `branch/end` 在要结束的那条线上的序号。

`source` 是现有的 `BranchSource`，与 `branch/create` 用的是同一个类型，所以创建和结束以同样的方式记录发起方：

| 来源 | 发起方 |
| --- | --- |
| `{kind: "tool", turn_id, request_id, call_id}` | 要结束的那条线自己的一次工具调用：`branch` 工具的 `end` |
| `{kind: "host", operation_id}` | 一次宿主操作。可以是 App Server 的 `session.branch_end`，它像 `session.branch_create` 那样生成 `operation_id`（一个 uuid，[connection/tree.rs](../../../crates/dim-app-server/src/connection/tree.rs)），也可以是带自己 id 的插件，例如 Goal 的 `goal:evaluate:N` |
| `{kind: "user", operation_id}` | 用户经 `session.branch_end` 自己发起的结束，种类与用户线的 `branch/create` 相同；于是客户端的结束能与插件的结束区分开 |

`ancestor` 指向用自己那次结束发起连锁的线：也就是本身不带 `ancestor` 的那次结束。连锁结束的每条后代都带这条线，`source` 也从同一次结束复制，于是（source, ancestor）这对值指明一次结束操作：谁要求的，在哪条线上。自己要求结束的线有 `source`、没有 `ancestor`；已被连锁结束之前就提交了自己结束的后代，保留它自己的那对值。`branch/ended` 从这条线的 `branch/end` 复制 `source` 和 `ancestor`，并且总是写在结束的那条线自己的父分支上，取自目录。

**一个通用操作。** `TreeCommands.end(id, source)` 和 `Session::end_branch(id, source)` 只检查目标：它存在、未结束、不是根、不是当前主线，也不在当前主线的祖先链上。结束根就等于关闭 Session，而结束一条线会连带结束它下面的每一条，所以主线整条链都受保护。这些是关于目标的检查，不是关于调用方的。它们从不检查是谁在调用：用户、开线方和插件遇到的是同一组守卫，而结束当前主线永远要先移动主线。

- 结束一条已经结束的线，什么也不写，回复 `branch_ended`。
- 还在打开的线还不是线：它的创建仍可能失败，而一条线只能有一个终结事实，所以在它发布之前 `end` 回答 `unknown_branch`。在加载时结束一条线的开线方等 Session，而 Session 在恢复完成后才发布（§17 R1）。
- 关闭 Session 不算结束。
- "开线的一方管理它的线"是调用方之间的约定，不是运行时规则：
  - `branch` 工具的 `end` 不带目标，所以结束的是调用它的那条线，这是工具的设计；而当前主线会被拒绝，因为 Session 的对话在它上面；
  - 一条线想让它开过的某条线结束，就用 steer 请它结束，这是工具的约定。

顺序：

1. 入口在这条线上提交 `branch/end`。此后这条线不再接受新输入，也不再开出新线，它下面的线也不再开出新线：这些入口都以 `branch_ended` 失败。它下面的线仍可接受输入，直到它自己提交 `branch/end`（第 3 步）。
2. 这条线上的 TreeTimeline 消费 `branch/end`，转给 TimelineTree。
3. TimelineTree 先结束这条线的后代，走同样的步骤；无论多深，每条后代都复制这个 `source`，并把 `ancestor` 设为这条线。
4. 关闭时机：
   - `tool` 来源：在 `branch/end` 之后这条线第一次结束一轮时关闭，因为这次调用就在那一轮里；线空闲时立即关闭；
   - `host` 来源，以及因祖先结束而结束的线：立即关闭，中断当前这一轮。
5. `Branch::close()` 中断这条线、结算它的工作并卸载它（[branch.rs](../../../crates/dim-agent-runtime/src/branch.rs)）。
6. TimelineTree 最后在父分支上写 `branch/ended`。

在父分支上，BranchLine 只处理工具或用客户端开出的线；插件线归它们的创建者：

- 这样的线经工具结束自己时（`tool` 来源且没有 `ancestor`），只写名册更新。这条线的最终消息已经在 `branch/end` 之前以 steer 送到它的开线方。
- 这样的线的其他任何结束，写名册更新加上结束通知（§12.1），以 steer 写在同一条输入里。

`tool` 来源且没有 `ancestor` 时，客户端显示"自行结束"；其他情况显示结束它的操作；由连锁结束的线还会写明它的祖先，例如"ended with X"。

**恢复。** 父分支上有某个子分支的 `branch/ended` 时，这个子分支不活跃。恢复中的父分支读取历史时会报告这些事件（§5）。这条规则加入现有的几条：父分支不活跃、父分支上有这个子分支的 `branch/create-failed`、子分支的创建从未完成。

一条线有 `branch/end`、但父分支上没有对应的 `branch/ended` 时，不打开它。恢复按最深的线优先补完这类结束：在父分支上补写每条缺失的 `branch/ended`，`source` 与 `ancestor` 取自 `branch/end`，这条线自己的结果最后写。对于它下面的线，那个父分支同样不会打开，所以记录写进它的存储历史，它的固定头部随之延长。其下自己从未开始结束的线不补写 `branch/ended`：父分支已关闭、不会再重放，写上去也没有人读。恢复把每条这样的线按它停下的头部记为已结束，`source` 取自父分支的 `branch/create`，它的历史仍可读取（§1"结束之后"）。

**结束之后：**

- 这条线留在分支目录里，它的历史仍可从存储读取：经 `session.history`（v3），也经工具的 `read`。它的头部不再移动。
- fork 照旧复制目录行和根的事件。在 fork 点之前结束的线，在 fork 里也不活跃，因为它的 `branch/ended` 在复制的前缀里。
- 删除 Session 时，这条线随之删除。
- Session 关闭和空闲卸载都不算结束：各线恢复后仍未结束。

**插件开的线。** 每个开线的一方按自己的协议结束自己的线。Goal 在评审了结之后结束它的 Pursuit 评审者；结论之前的外部结束，它判为 `failed`（[Pursuit §13](plugins/Pursuit.md#13-结束评审者随-427)）。

## 11. `branch` 工具

工具接受一个按动作区分的输入，取代 `{prompt, at?}`。风险仍是 `session`，Approval 照旧放行每次调用。

| 动作 | 输入 | 谁 | 作用 | 结果文本 |
| --- | --- | --- | --- | --- |
| `create` | `prompt`、`at?`、`idleNotice?`（`steer` \| `none`） | 当前主线 | 开一条切在 `at` 的工具线（默认与现在相同：在本次请求的模型开始之前）。空闲通知默认 `steer`。这个 Session 已经把工具开着的线开到上限时拒绝（§6.1） | `Line created: C` |
| `message` | `to`、`content`、`mode?`（`append` \| `steer`） | 一条线 → 它开过的线；一条线 → 它被切出来的那条线 | 在 `to` 上写一条 `input/message` | `Sent to C.` |
| `read` | `branch`、`after?`、`limit?`（1–200） | 一条线，读它被切出来的那条线或它开过的线 | 从存储读一页；不给 `after` 时从调用方的游标开始（§6） | 这一页的 JSON：items、`nextCursor`、`done` |
| `end` | `content` | 一条不是当前主线的线，对它自己 | 先给开它的那条线发最终消息（steer），再调用 `TreeCommands.end(C, source)`，`source` 就是这次调用 | `Line C is ending; your final message was sent to P.` |

输入 schema 为 `{action, prompt?, at?, idleNotice?, to?, content?, mode?, branch?, after?, limit?}`，`action` 必填，`additionalProperties: false`。`idleNotice` 只对 `create` 有效。

拒绝以错误结果返回（代码在 `structuredContent`），什么也不写：

| 情形 | 文本 | 代码 |
| --- | --- | --- |
| 在不当前主线的线上 `create` | `Only the primary line opens lines.` | `branch_not_primary` |
| 这个 Session 已经把工具开着的线开到上限时 `create` | 写明上限的拒绝 | `branch_limit_reached` |
| 在亲缘之外 `message` 或 `read` | `You can message or read only the lines you opened and the line you were cut from.` | `branch_not_kin` |
| 对插件线 `message`、或读插件线 | `Line C was not opened with this tool; it follows its own protocol.` | `branch_not_tool_line` |
| 向已结束的线发消息，或与结束发生竞争 | `Line C has ended. Read it to see what it did.` | `branch_ended` |
| 在根、当前主线、主线的祖先或插件线上 `end` | `Only a line that opened this one, and that is not the primary line, can end it with this call.` | `branch_cannot_end` |
| `read` 的 `after` 超过目标的头部 | `after A is past line T's head H; a read starts at or below H` | 无：带错误结果，不带代码 |

这些拒绝文本只陈述规则，不多写。模型在收到拒绝后该怎么做属于策略，与其余分支文本的指引一起，留给提示词设计（§12）决定。

这些拒绝是工具自己的规则。`TreeCommands.end` 本身只检查目标（§10）。如果线已经提交了自己的 `branch/end`，再来的 `end` 取 `branch_ended` 一行，不会再写一条最终消息：第一次调用的消息就是这条线最后的话。

## 12. 提示词

### 12.1 上下文映射

`C` 是这条线，`P` 是它被切出来的那条线（根开出的线，`P` 就是 `main`）。标记内的内容按 TimelineTree 现在的方式转义（`context_block`）。

| 写入 | 位置、key | 模式 | 文本 |
| --- | --- | --- | --- |
| 身份，工具线 | `C`，开启输入的第一个块，`branch:line` | Queue | `<dim-branch-line branch="C" parent="P" source="tool">` / `You are line C of this session. Line P opened you at this point of its history to work in parallel with it. You are the same agent as P: everything above is your own history, and you continue from here on your own line. P keeps working on its line; you can read its work after this point, and it can read yours. Messages above that were addressed to P, such as its list of open lines, are P's, not yours.` / 空行 / `Your purpose, as P wrote it:` / `<prompt>` / 空行 / `How you work with P:` / `- Work on this purpose only. You cannot open lines unless you are the primary line, and you are not while this block is your latest one; if more parallel work is needed, tell P.` / `- Read P with the branch tool (action read, branch "P") when what P did or decided since your cut changes your work; the parent block reports when P has moved. Message P only for what it must act on: a decision that changes its work, a blocker only the user can settle, a conflict with another line, or a change to your purpose. Use mode "steer" for what P must act on now, "append" for what P should see at its next step. Do not send P a progress report it can read, and do not page through P's whole stream: read the page that answers your question, then act.` / `- You share the workspace with P and its other lines. Change only the files P gave you; before changing any other file, tell P which. If edit or write fails because the file was modified after it was last read, another line may have changed the file: read it again and tell P before you continue.` / `- The user can talk to you directly. Answer the user, and ask with ask_user when only the user can decide.` / `- When your purpose is done, or cannot be done, call the branch tool with action "end" and content saying what you found, did, changed and left undone. If you already told P that in a message, your end content carries only what is new. Read P once more before you end if the parent block says P has moved, so your last word still holds. That is your last action; P receives it.` / `</dim-branch-line>` |
| 身份，用户开的线 | `C`，开启输入的第一个块，`branch:line`。用户的 prompt 是这条输入的 `user` | Queue | `<dim-branch-line branch="C" parent="P" source="user">` / `You are line C of this session, opened by the user from line P at this point of its history. You are the same agent as P: everything above is your own history, and you continue from here on your own line, in parallel with P. You can read P's work after this point. Unless the user makes this line the primary line, you cannot open new lines with create, and you cannot call end on yourself. Messages above that were addressed to P are P's, not yours. The message after this one is from the user for this line.` / `</dim-branch-line>` |
| 身份，插件线 | `C`，开启输入的第一个块，`branch:line`。创建者的 prompt 是这条输入的 `user` | Queue | `<dim-branch-line branch="C" parent="P" source="host">` / `You are line C of this session, opened from line P at this point of its history. You are the same agent as P: everything above is your own history, and you continue from here on your own line, in parallel with P. You can read P's work after this point. Messages above that were addressed to P are P's, not yours. The message after this one says what this line is for.` / `</dim-branch-line>` |
| 名册 | `O`，`branch:roster` | Append | `<dim-branch-lines branch="O">` / `Lines you opened that are still open. Each is you, working in parallel from the point you opened it; it can read your work after that point, and you can read its.` / `- C1: <目的的第一行，最多 120 个字符>` / … / ``They message you when they find something, need you, or end. Use the branch tool to message or read them, and the `wait` tool to wait for them; end the wait when your lines have nothing left to tell you.`` / `</dim-branch-lines>` |
| 名册，没有未结束的线 | `O`，`branch:roster` 退役 | Append | `<dim-branch-lines branch="O">` / `All lines you opened have ended.` / `</dim-branch-lines>` |
| 父线块 | `C`，`branch:parent` | Append | `<dim-branch-parent branch="P" head="H" after="A">` / `Line P has new activity: it was at H when this was written, and you have read it to A. Read what is new with the branch tool (action read, branch "P"); with no after, the read continues from A. Read it when what P did changes what you do, and before your final message; do not page through all of P's work.` / `</dim-branch-parent>` |
| 父线块，嵌套线开启时 | `C`，开启输入，身份块之后，`branch:parent` | Queue | `<dim-branch-parent branch="P" head="H" after="H">` / `Line P has no new activity since you were cut from it at H. Parent blocks above this point were written for P and are not yours.` / `</dim-branch-parent>` |
| 消息 | 接收方，无 key | `append` 或 `steer` | `<dim-branch-message from="C" to="P">` / `<content>` / `</dim-branch-message>` |
| 最终消息 | `P`，无 key | Steer | `<dim-branch-message from="C" to="P" final="true">` / `<content>` / 空行 / `Line C ends with this message.` / `</dim-branch-message>` |
| 空闲通知 | `P`，无 key | Steer | `<dim-branch-idle branch="C" status="completed\|failed\|cancelled">` / `Line C's turn ended (<status>) without a message to you, and C is waiting. Read it if you need its state; message it with mode "steer" to give it more work, or leave it.` / `</dim-branch-idle>` |
| 结束通知 | `P`，无 key，与名册在同一条输入里；`P` 开过的线除自己的 `end` 之外的任何结束 | Steer | `<dim-branch-ended branch="C">` / `Line C was ended from outside its own line, by the user or a host operation. It no longer receives messages; read it if you need what it did.` / `</dim-branch-ended>` |
| 主线状态说明，新的主线（切自某条父线） | `C`，`branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="tool" primary="true">` / `You are line C of this session. Line P opened you at this point of its history to work in parallel with it. You are the same agent as P: everything above is your own history, and you continue from here on your own line. P keeps working on its line; you can read its work after this point, and it can read yours. Messages above that were addressed to P, such as its list of open lines, are P's, not yours.` / `空行` / `This line is now the session's primary line, switched from line O. You can still read P's work after your cut, and if you were opened with create you can still message P. While this block is your latest one you can open new lines with create, you send no automatic idle notices to P, and you cannot call end on yourself. Line O stays open and keeps coordinating the lines it already opened.` / `空行` / `Your purpose, as P wrote it:` / `<prompt>` / `The user switched here to continue directly from this line's context and approach: the user's request takes precedence over this purpose, which still says what this line is for.` / `空行` / `How you work with P:` / `- Work from the purpose above and from what the user asked for here. You are the primary line while this block is your latest one, so you can open lines; open one only when the four conditions for opening one hold, and not to hand this purpose off.` / `- Read P with the branch tool (action read, branch "P") when what P did or decided since your cut changes your work; the parent block reports when P has moved. Message P only for what it must act on: a decision that changes its work, a blocker only the user can settle, a conflict with another line, or a change to your purpose. Use mode "steer" for what P must act on now, "append" for what P should see at its next step. Do not send P a progress report it can read, and do not page through P's whole stream: read the page that answers your question, then act.` / `- You share the workspace with P and its other lines. Change only the files P gave you; before changing any other file, tell P which. If edit or write fails because the file was modified after it was last read, another line may have changed the file: read it again and tell P before you continue.` / `- The user can talk to you directly. Answer the user, and ask with ask_user when only the user can decide.` / `- When your purpose is done, or cannot be done, message P with what you found, did, changed and left undone and keep working: while this line is primary you cannot call end on yourself.` / `空行` / `How you work as the primary line:` / `- Carry this line's main work forward yourself; open a new line only when the four conditions for opening one hold again, not to hand off your current task.` / `- Line O stays open in the shared workspace and keeps coordinating any lines it already opened. Avoid redoing work assigned to lines opened before your cut. If edit or write fails because the file was modified after it was last read, another line may have changed that file: read it again before editing.` / `</dim-branch-line>` |
| 主线状态说明，新的主线（用户开的线） | `C`，`branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="user" primary="true">` / `You are line C of this session, opened by the user from line P at this point of its history. You are the same agent as P: everything above is your own history, and you continue from here on your own line, in parallel with P. You can read P's work after this point. Unless the user makes this line the primary line, you cannot open new lines with create, and you cannot call end on yourself. Messages above that were addressed to P are P's, not yours. The message after this one is from the user for this line.` / `空行` / `The user has now made this line the session's primary line, switched from line O, so the identity above applies as it is written: you can open new lines with create, and you cannot call end on yourself. Line O stays open and keeps coordinating the lines it already opened.` / `空行` / `How you work as the primary line:` / `- Carry this line's work forward yourself; open a new line only when the four conditions for opening one hold again.` / `- Line O stays open in the shared workspace and keeps coordinating any lines it already opened. If edit or write fails because the file was modified after it was last read, another line may have changed that file: read it again before editing.` / `</dim-branch-line>` |
| 主线状态说明，新的主线（根） | `C`，`branch:line` | Append | `<dim-branch-line branch="main" primary="true">` / `You are line main, the root line of this session, and this line is the session's primary line again (switched from line O). The root line never ends; closing the session ends it. While this block is your latest one you can open new lines with create and continue coordinating any open lines you opened.` / `空行` / `How you work as the primary line:` / `- Carry the user's current work forward here; open a new line only when the four conditions for opening one hold again.` / `- Line O stays open in the shared workspace and keeps coordinating any lines it already opened. If edit or write fails because the file was modified after it was last read, another line may have changed that file: read it again before editing.` / `</dim-branch-line>` |
| 主线状态说明，旧的主线（不是线 N 的祖先） | `C`，`branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="tool" primary="false">` / `You are line C of this session. Line P opened you at this point of its history to work in parallel with it. You are the same agent as P: everything above is your own history, and you continue from here on your own line. P keeps working on its line; you can read its work after this point, and it can read yours. Messages above that were addressed to P, such as its list of open lines, are P's, not yours.` / `空行` / `Line N is now the session's primary line; this line stays open and keeps the lines it opened. You can no longer open new lines with create.` / `空行` / `Your purpose, as P wrote it:` / `<prompt>` / `空行` / `How you work with P:` / `- Work on this purpose only. You cannot open lines unless you are the primary line, and you are not while this block is your latest one; if more parallel work is needed, tell P.` / `- Read P with the branch tool (action read, branch "P") when what P did or decided since your cut changes your work; the parent block reports when P has moved. Message P only for what it must act on: a decision that changes its work, a blocker only the user can settle, a conflict with another line, or a change to your purpose. Use mode "steer" for what P must act on now, "append" for what P should see at its next step. Do not send P a progress report it can read, and do not page through P's whole stream: read the page that answers your question, then act.` / `- You share the workspace with P and its other lines. Change only the files P gave you; before changing any other file, tell P which. If edit or write fails because the file was modified after it was last read, another line may have changed the file: read it again and tell P before you continue.` / `- The user can talk to you directly. Answer the user, and ask with ask_user when only the user can decide.` / `- When your purpose is done, or cannot be done, call the branch tool with action "end" and content saying what you found, did, changed and left undone. If you already told P that in a message, your end content carries only what is new. Read P once more before you end if the parent block says P has moved, so your last word still holds. That is your last action; P receives it.` / `空行` / `How you work while another line is primary:` / `- Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line.` / `- You share the workspace with line N; do not edit files outside your remaining scope unless asked, and if edit or write fails because the file was modified after it was last read, read the file again before continuing.` / `</dim-branch-line>` |
| 主线状态说明，旧的主线（是线 N 的祖先） | `C`，`branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="tool" primary="false">` / `You are line C of this session. Line P opened you at this point of its history to work in parallel with it. You are the same agent as P: everything above is your own history, and you continue from here on your own line. P keeps working on its line; you can read its work after this point, and it can read yours. Messages above that were addressed to P, such as its list of open lines, are P's, not yours.` / `空行` / `Line N is now the session's primary line; this line stays open and keeps the lines it opened. You can no longer open new lines with create.` / `空行` / `Your purpose, as P wrote it:` / `<prompt>` / `空行` / `How you work with P:` / `- Work on this purpose only. You cannot open lines unless you are the primary line, and you are not while this block is your latest one; if more parallel work is needed, tell P.` / `- Read P with the branch tool (action read, branch "P") when what P did or decided since your cut changes your work; the parent block reports when P has moved. Message P only for what it must act on: a decision that changes its work, a blocker only the user can settle, a conflict with another line, or a change to your purpose. Use mode "steer" for what P must act on now, "append" for what P should see at its next step. Do not send P a progress report it can read, and do not page through P's whole stream: read the page that answers your question, then act.` / `- You share the workspace with P and its other lines. Change only the files P gave you; before changing any other file, tell P which. If edit or write fails because the file was modified after it was last read, another line may have changed the file: read it again and tell P before you continue.` / `- The user can talk to you directly. Answer the user, and ask with ask_user when only the user can decide.` / `- When your purpose is done, or cannot be done, message P with what you found, did, changed and left undone and keep working: line N is below this line, and ending this line would end line N, so you cannot call end on yourself.` / `空行` / `How you work while another line is primary:` / `- Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line.` / `- You share the workspace with line N; do not edit files outside your remaining scope unless asked, and if edit or write fails because the file was modified after it was last read, read the file again before continuing.` / `</dim-branch-line>` |
| 主线状态说明，旧的主线（用户开的线，不是线 N 的祖先） | `C`，`branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="user" primary="false">` / `You are line C of this session, opened by the user from line P at this point of its history. You are the same agent as P: everything above is your own history, and you continue from here on your own line, in parallel with P. You can read P's work after this point. Unless the user makes this line the primary line, you cannot open new lines with create, and you cannot call end on yourself. Messages above that were addressed to P are P's, not yours. The message after this one is from the user for this line.` / `空行` / `Line N is now the session's primary line; this line stays open and keeps the lines it opened. You can no longer open new lines with create. You cannot call end on yourself; the user ends this line.` / `空行` / `How you work while another line is primary:` / `- Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line.` / `- You share the workspace with line N; do not edit files outside your remaining scope unless asked, and if edit or write fails because the file was modified after it was last read, read the file again before continuing.` / `</dim-branch-line>` |
| 主线状态说明，旧的主线（用户开的线，是线 N 的祖先） | `C`，`branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="user" primary="false">` / `You are line C of this session, opened by the user from line P at this point of its history. You are the same agent as P: everything above is your own history, and you continue from here on your own line, in parallel with P. You can read P's work after this point. Unless the user makes this line the primary line, you cannot open new lines with create, and you cannot call end on yourself. Messages above that were addressed to P are P's, not yours. The message after this one is from the user for this line.` / `空行` / `Line N is now the session's primary line; this line stays open and keeps the lines it opened. You can no longer open new lines with create. Line N is below this line, so ending this line would end line N as well.` / `空行` / `How you work while another line is primary:` / `- Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line.` / `- You share the workspace with line N; do not edit files outside your remaining scope unless asked, and if edit or write fails because the file was modified after it was last read, read the file again before continuing.` / `</dim-branch-line>` |
| 主线状态说明，旧的主线（根） | `C`，`branch:line` | Append | `<dim-branch-line branch="main" primary="false">` / `You are line main, the root line of this session. Line N is now the session's primary line. The root line never ends; closing the session ends it. You can no longer open new lines with create, and the lines you already opened stay yours to coordinate.` / `空行` / `How you work while another line is primary:` / `- Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line.` / `- You share the workspace with line N; do not edit files outside your remaining scope unless asked, and if edit or write fails because the file was modified after it was last read, read the file again before continuing.` / `</dim-branch-line>` |

**为什么状态说明要重述身份。** 每段说明都是同一个 `branch:line` 键的后续块，因此它替换接收它的那条线出生时的身份块；压缩只重放最新的块，所以一条无法从当前块里说出自己的父线、来源、目的和结束规则的线，会在余下的生命里一直缺少这些事实。因此说明逐字重复出生块里的句子，只改动这次挪动真正改变的那几句：这条线能开什么、它还给 P 发什么、它能不能 `end`。写入方知道这条线属于哪种情形，直接写那种情形的那一行；没有任何说明写成条件句，也没有任何说明对结束规则含糊其辞。

**哪一行用于哪条线。** 这里的“祖先”指用户挪到的那条线的祖先，这是常见情形：旧主线开出的线就是它的祖先。

| 收到说明的线 | 用哪一行 |
| --- | --- |
| 用户挪到的线，由某条线开出 | “新的主线（切自某条父线）” |
| 用户挪到的线，由用户开出 | “新的主线（用户开的线）” |
| 用户挪到的线，根 | “新的主线（根）” |
| 旧主线，不是新主线的祖先，由某条线开出 | “旧的主线（不是线 N 的祖先）”：逐字保留出生块的第 5 条，它按自己的来源用 `end` 结束自己 |
| 旧主线，不是新主线的祖先，由用户开出 | “旧的主线（用户开的线，不是线 N 的祖先）”：它的身份本就拒绝 `end`，说明重述这一点：用户结束这条线 |
| 旧主线，是新主线的祖先，由某条线开出 | “旧的主线（是线 N 的祖先）”：第 5 条改说级联，不再用 `end` |
| 旧主线，是新主线的祖先，由用户开出 | “旧的主线（用户开的线，是线 N 的祖先）”：它的身份本就拒绝 `end`，说明补上原因 |
| 旧主线，根 | “旧的主线（根）”：根永不结束 |

| 工具描述 | 每条线 | — | `Work with the other lines of this session. A line is you: it starts from your history at the point it is opened, shares this workspace, and works in parallel on its own line; the user can talk to any line directly or make it the session's primary line. It is not a helper with its own memory, and it owes you no result; you and it keep each other informed by reading each other's work and messaging what the other must act on. On the primary line you coordinate: answer a line's steer and act on what it asks for; leave a line's part to that line instead of redoing it; read a line's work instead of waiting for a report.` / `- create (primary line only): open a line to work on prompt in parallel. Open a line only when all four of these hold: the work needs this conversation's context, or its approach is worth the user seeing and taking over; after opening it you have other work to do here right away rather than waiting for it; the part you hand over is multi-step work, not two or three quick actions; and the two lines will not write the same files at the same time. When the user asks for a line, open it. These are not line-shaped: a long command runs in the background instead of taking a line; work that needs none of this conversation's context, or needs different instructions, belongs to another member; one task pursued to completion stays here, or uses a goal; waiting for something outside the session uses an event subscription. Write prompt as a brief to yourself: the purpose in one line, what you decided in this response that the line needs, which files or area are its own (or that it is read-only), and when it should tell you or end. Returns the line's id.` / `- message: send content to a line you opened, or to the line that opened you. Send a decision, a blocker, or a conflict, not a progress report the receiver can read. Mode "append" (default) reaches the receiver at its next step without interrupting it and does not wake an idle line. Use "steer" when the receiver must act now or is idle.` / `- read: show a page of the activity of the line you were cut from, or of a line you opened, also after it ended. With no after, a read continues after your last read of that line: from your cut for the line you were cut from, and from its first event for a line you opened. Pass after to read from another point. Do not page through a whole line.` / `- end: on a line opened with create, end your own line unless it is the primary line. content is your final message to the line that opened you: what you found, did, changed and left undone. It is your last action.` / `A line reads the line it was cut from, and the lines it opened. Messages pass only between a line and the lines it opened.` |

这些文本取代代码里现有的委派措辞：`first_input` 里的 `dim-branch-task` 块（"That branch delegated this task to you"）和工具描述（"works on prompt as a task you delegate"）。各段文本的类别见 [§12.4](#124-文本及其类别)。

### 12.2 交给提示词层的事项（[#429](https://github.com/arcships/dim/issues/429)）

上面的文本是初版，归 TimelineTree 插件所有。下面这些事项属于面向模型的提示词层：

- 开线方怎样给它开的线写说明，以及协调对开线方意味着什么：分配区域、回应 steer、结束自己的等待、不重做线上的工作。
- **Goal 的措辞（归 Goal）。** 一条线不是被委派的工作（§1）：目标块的句子、续跑块的句子，以及 [Goal §3.4](plugins/Goal.md#34-新分支清除继承的目标) 和 Goal §5 子分支那一行的陈述。这些文本见 [Goal §A.5](plugins/Goal.md#a5-模型可见文本)；任一句的改动都是一次策略改动，按 [Goal §A.5.2](plugins/Goal.md#a52-评估) 判定。这些文本归 Goal。

回答这些事项的需求见 [面向模型的提示词](model-prompts.md)。

### 12.3 评估场景

场景放在仓库里，一个场景一个目录，目录里是 `scenario.json` 和它旁边的起始 `workspace/`：十四个在 [`xtask/scenario-eval/timeline-branches/`](../../../xtask/scenario-eval/timeline-branches/) 下，各自声明 131,072 tokens 的窗口；`identity-after-compaction` 在 [`xtask/scenario-eval/timeline-branches-compact/`](../../../xtask/scenario-eval/timeline-branches-compact/) 下，声明 16,000，以便发生压缩。`budget.window` 是校验而不是设置：Session 声明的窗口与之不同时，这次运行在第一轮之前就失败，所以重跑需要声明的窗口恰好等于场景窗口的模型，什么也不声明的模型回退到 128,000，同样失败。这些都在本地用真实模型运行，结果写在实现 PR 里。CI 只校验行为。

`main-moved-on` 是父线块（§7.1.1）的场景：一条线在切点之前有工作，开一条线，然后继续工作；新开的线在回答或结束之前必须读取它的开线方在切点之后做的事，并且不能逐页读遍开线方的全部活动。它是 [§12.4](#124-文本及其类别) 父线块策略句的场景。

`identity-after-compaction` 沿用 `redone-work` 的消息和 workspace，`flooding` 与 `never-ending` 共用同一个 workspace：这两对保持完全相同，改动其中一份而不同步另一份时，读取两个提交目录的无模型测试会失败。

**哪些场景是“该开”的。** 三个，它们就是 [§12.4](#124-文本及其类别)提升门禁测的那三个：`long-work-and-a-flaky-test`、`refactor-and-a-pasted-report` 和 `switch-then-open-lines`。这三个场景里开线明显优于串行，而用户消息从不提“线”，也从不要求开线：一次运行在这三个场景里怎么做，是模型自己的判断。其余场景要么禁止开线、要么要求开线，要么测一条线已经存在之后的协调；在那些场景上升高开线率并不好，所以它们只受“无退化”门禁约束。该开场景的通过条件从不要求开过线：一次正确的串行运行同样通过，开线率是门禁 2 单独统计的量，与通过条件分开；反方向的压力由 `over-splitting` 和 boundary 行维持。

标着 `new` 的六个场景随主线一并加入，因为措辞现在说明了何时开线，而一条线的角色可以在会话中途改变。每个场景声明它的 `message`、起始 `workspace/`，需要切换的再声明一个 `switch` 步骤：该线第一轮结束后，运行器对步骤指明的那条线调用 `session.branch_primary`，然后才发送下一条输入。没有 `switch` 步骤的场景照旧运行。运行器是 `xtask/src/scenario_eval.rs`；其中 `submit_params` 不再发送 `branchId`，而是省略它，于是切换之后的输入按 Session 自己的缺省目标到达主线。每个 `new` 行都逐字引用它的消息：消息就是场景的第一条输入，任何文件都不给它加词。

| 场景 | 场景目录 | 用户说什么、workspace 里有什么 | 关注什么 | 四道条件，按构造成立 | 通过条件 |
| --- | --- | --- | --- | --- | --- |
| 线把自己当成主线 | — | — | 不是主线的线去开线、把不属于自己的名册当成自己的，或去协调不是它开的线 | — | — |
| 重复劳动 | `timeline-branches/redone-work` | 两个互不相干的子系统，消息里写明它们不共享代码、不改同一批文件 | 开线方重做一条线正在做的事，或者两条线做同一件事 | — | 没有重复劳动，两个子系统的测试都通过 |
| 过时的认识 | — | — | 一条线依据切点之后已经改变的文件或决定行事，却没有重新读取 | — | — |
| 父线动了 | `timeline-branches/main-moved-on` | 一条线先工作，开一条线，然后继续工作 | 一条线在回答或结束前没有读取父线在切点之后提交的活动，或者逐页读遍了父线的全部活动 | — | 这条线的回答引用了它的开线方在切点之后做的事 |
| 刷屏 | `timeline-branches/flooding` | 已经开出两条线，各改各的文件 | append 或 steer 消息重复接收方已经知道的内容 | — | 没有消息重复接收方已经持有的内容 |
| 过度拆分 | `timeline-branches/over-splitting` | 两处常量改名并跑一次单测 | 为很短或必须按顺序做的步骤开线 | — | 没有开出任何线 |
| 不结束 | `timeline-branches/never-ending` | 一条线完成了目的，它的开线方继续工作 | 一条线已经做完，却不调用 `end` | — | 做完的线自己结束 |
| 兄弟中转 | `timeline-branches/sibling-relay` | 两条线的工作本该是一条线，开线方在中间 | 用开线方在本该合成一条线的几条线之间传话 | — | 两条线直接对话，或这项工作在一条线上完成 |
| 压缩后的身份 | `timeline-branches-compact/identity-after-compaction` | 开线方和某条线各自压缩之后 | 每条线忘记自己是哪条线，或把开线方的名册当成自己的 | — | 每条线仍能说出自己的 id、父线和名册 |
| 来回空转 | `timeline-branches/ping-pong` | 两条线本可以闲置 | 不推进工作的 steer 往复 | — | 没有不推进工作的 steer 链 |
| 问用户 | `timeline-branches/asking-the-user` | 某条线需要一个只有用户能做的决定 | 该问用户时，线直接问用户，而不是绕道开线方 | — | 这条线用一次提问直接问用户 |
| **长任务与漏洞排查并行**（新，该开） | `timeline-branches/long-work-and-a-flaky-test` | **消息：**Two jobs today. First, add cursor pagination to GET /items in api/list.py: a cursor and a limit parameter, a stable order, its own tests in tests/test_list.py, and the README section. Second, tests/test_retry.py::test_retry_happens fails about once in thirty runs on CI only: reproduce it, find the cause, and write it up in docs/flaky-retry.md with the evidence. Neither job waits for the other. `api/list.py`、`README.md`、`tests/test_list.py` 归第一件；`src/retry.py`、`tests/test_retry.py`、`docs/flaky-retry.md` 归第二件；测试用 `python3 -m unittest discover -s tests`。 | 是否有一条线去做那个偶发失败的排查，而主线在这里实现分页；以及两条线随后是否改了对方的文件 | 1，排查从这条消息里的报告（三十次 CI 运行里失败一次）出发，接口的形状也在这里定下，两者都不在任何文件里；2，分页自成一件工作，不需要排查的答案，所以开线之后这条线并不需要等待；3，复现三十次一遇的失败、找出原因并带着证据写下来需要好几步；4，排查与分页的文件没有交集 | 分页可用且 `tests/test_list.py` 通过，`docs/flaky-retry.md` 用显示原因的证据说出原因，两件事的文件都没有被对方的工作改动。在一行上做完两件事的运行同样通过 |
| **重构与贴进来的报告并行**（新，该开） | `timeline-branches/refactor-and-a-pasted-report` | **消息：**Two things before the release. Move the CSV writer out of report/export.py into report/writer.py with its own tests in tests/test_writer.py, keeping the public functions and the golden files in tests/golden/ passing; that is several files and I want it finished today. Separately, the Nightly job sent this report: `Traceback (most recent call last): File "runbooks/nightly.py", line 88, in flush / rows = list(self._pending) / RuntimeError: ordered dict mutated during iteration`. Work out what it means, reproduce it, and write the cause and the fix in docs/incident-4471.md with the evidence. `report/export.py`、`tests/test_export.py`、`tests/golden/*.csv`、`runbooks/nightly.py`，以及一开始并不存在的 `docs/incident-4471.md`。 | 是否在一条单独的线上排查这份报告，而重构在这里继续；以及两条线随后是否改了对方的文件 | 1，排查从这条消息里贴出的 traceback 出发，切点会把它带进那条线，而没有任何文件持有它；2，重构是几个文件的工作，不需要排查的答案；3，读懂 nightly 的 flush 路径、复现那次迭代中的修改、带着证据写下来需要好几步；4，`report/` 连同 `tests/golden/` 与 `runbooks/` 没有交集 | 重构完成，公开函数与基准文件继续通过，`docs/incident-4471.md` 用显示原因的证据说出原因和修法。在这里同时做完两件事的运行同样通过 |
| **一个文件，两套方案**（新，boundary） | `timeline-branches/same-file-tradeoff` | **消息：**We need to pick a rate limiter for src/rate_limiter.py. Read NOTES.md for the burst requirements, compare a token bucket with a sliding window, tell me which fits and why, and only then change the file. `src/rate_limiter.py`、它的测试，以及写了突发流量要求的 `NOTES.md`。 | 两条线都写 `src/rate_limiter.py`，或在给出结论之前就改了文件 | — | 两套方案都按突发用例比较过，且给出结论时 `src/rate_limiter.py` 未被改动。比较两套方案、先给结论再改文件在一条线上也合理，所以这里允许开线，但不要求开线 |
| **长命令不是一条线**（新，boundary） | `timeline-branches/background-command-not-branch` | **消息：**Two things, both needed today: run python3 scripts/slow_audit.py and tell me what its report says, and fix the leap-year bug in src/date_util.py with a test. `scripts/slow_audit.py` 跑五秒并打印报告；`src/date_util.py` 有一个闰年单行 bug；`tests/test_date_util.py` 没有覆盖它的用例。 | 模型是否为一条命令就花掉一条线（那属于普通的命令工作），以及是否为修 bug 也凭空开线 | — | 脚本跑完且报告的内容被读到，bug 用一个先失败后通过的测试修好，并且没有为那条命令开线。一条线都不开同样通过 |
| **沿着更好的线继续**（新，切换） | `timeline-branches/switch-continue-directly` | **消息：**Have a line work out how src/config_loader.py should merge layered defaults, what wins on a list, and what the tests must cover. At the same time, check schema.toml for keys that conflict with each other. I will tell you here which of the two to build. `src/config_loader.py`、`schema.toml` 以及两者旁边的测试。该线第一轮结束后，运行器把主线挪到它，下一条消息说这套设计更好并要求在这里实现。 | 新主线是自己在做这件事，又转给一条新线，还是对自己 `end` | — | 新主线自己改 `src/config_loader.py` 且测试通过，这个实现步骤没有新开线，这条线没有对自己 `end` |
| **新主线能开线**（新，切换，该开） | `timeline-branches/switch-then-open-lines` | **消息：**Have a line diagnose why tokens expire early in auth/token.py, with a failing test that shows it. While it does, read README.md and tell me which install path the docs describe. `auth/token.py`、`cli/login.py`、`audit/exporter.py` 及各自测试。该线第一轮结束后运行器把主线挪到它；下一条消息要求在这里修好，并另做两项增加：`cli/login.py` 的 `--leeway`（解析、读取它的刷新路径、测试和帮助文本）与 `audit/exporter.py` 的 CSV 输出（写入器、`--format` 参数、测试和帮助文本）。 | 新主线是否为两项独立增加开线，以及旧主线是否越权插手 | 1，两项增加都建立在这条线自己产出的诊断之上，切点会把它带进来；2，每项增加自成一件工作，不需要另一项的答案，所以开一条不会让这条线空等；3，每项都是参数、代码路径、测试和帮助文本，需要好几步；4，`cli/login.py` 与其测试、`audit/exporter.py` 与其测试没有交集 | 过早过期的 bug 带测试修好，两项增加连同测试和帮助文本都可用，三套测试都通过，旧主线没有开线也没有改动任何新文件。在一行上做完所有事的运行同样通过 |

场景目录为 `—` 的一行在这个场景集里没有对应场景；没有通过条件的那一行也没有对应场景。

### 12.4 文本及其类别

机制中每一段面向模型的文本及其类别（[面向模型的提示词 §4](model-prompts.md#4-协议与策略)）。确切文本见 [§12.1](#121-上下文映射)；这里按开头几个词指认每一句，混合文本逐句标注。

**协议。**

- **标记名与属性。** `dim-branch-line` 带 `branch`、`parent`、`source`，主线状态说明还带 `primary`（`true` 或 `false`），key 为 `branch:line`；`dim-branch-lines` 带 `branch`，key 为 `branch:roster`；`dim-branch-parent` 带 `branch`、`head` 和 `after`，key 为 `branch:parent`；`dim-branch-message` 带 `from`、`to`，最终消息再带 `final`；`dim-branch-idle` 带 `branch` 和 `status`（`completed`、`failed` 或 `cancelled`）；`dim-branch-ended` 带 `branch`。模式用的是 InputQueue 的：身份是 Queue，名册、父线块和主线状态说明是 Append，通知和最终消息是 Steer。
- **身份块的事实句。** 工具线里，"You are line C of this session" 到 "are P's, not yours."，以及标签 "Your purpose, as P wrote it:"；用户开的线和插件线里，整句，`source` 属性指明是哪一种。标签后的 purpose 是创建者自己的话，按内容承载。
- **名册的事实句。** "Lines you opened that are still open. Each is you …" 与 "All lines you opened have ended."；列出的每一条带线的 id 和目的的第一行。
- **父线块的事实句。** "Line P has new activity: it was at H when this was written, and you have read it to A."、"Read what is new with the branch tool (action read, branch \"P\")" 中的动作与字段名，以及规则 "with no after, the read continues from A."。
- **通知的事实句。** "Line C's turn ended (<status>) without a message to you, and C is waiting." 与 "Line C was ended from outside its own line, by the user or a host operation. It no longer receives messages"。
- **主线状态说明的事实句。** 给新主线的说明写出它的 id、父线和来源（取自它重复的出生块）、出生块带有目的时的目的、它切自哪条线，以及在这段块是该线最新块期间成立的规则：它可以开线，它不向开它的线发自动空闲通知，它不能对自己 `end`。给旧主线的说明写出现在是哪条线是主线、这条线继续开着并保留它开过的线、它不能再开线，以及它按自己情形适用的结束规则：新主线的祖先不能 `end`，因为 `end` 会级联向下，说明写明这一点；不是祖先的线逐字保留来源给它的结束规则：工具线用 `end` 结束自己，用户开的线则被告知它不能对自己 `end`、由用户来结束；根永不结束。每段说明都是同一个 `branch:line` 键的后续块，因此替换出生身份：它逐字重复那份身份的句子，只改动挪动真正改变的部分（§12.1）。
- **branch 工具的动作与模式。** 动作 `create`、`message`、`read`、`end` 及各自做什么；`mode` 为 `append` 或 `steer` 及它对接收方的作用；每个动作接收的字段，`action` 必填且不接受别的字段；`read` 的目标规则及其游标规则（"With no after, a read continues after your last read of that line"）；以及陈述保证的句子："Returns the line's id"、"Pass after to read from another point"、"It is your last action"、"A line reads the line it was cut from, and the lines it opened. Messages pass only between a line and the lines it opened."。
- **结果与拒绝。** `Line created: C`、`Sent to C.`、页 `{items, nextCursor, done}`、`Line C is ending; your final message was sent to P.` 与 `Line C ends with this message.`；拒绝码 `branch_not_primary`、`branch_limit_reached`、`branch_not_kin`、`branch_not_tool_line`、`branch_ended`、`branch_cannot_end` 及各自的文本；`after A is past line T's head H; a read starts at or below H`，即读取在 `after` 超过目标头部时返回的错误结果，不带代码；以及 `Invalid branch arguments: …`。
- **指南条目。** 每个条目标明它的标记是什么、承载谁的话或哪些事实，以及它是状态或信息、绝不是请求。`<dim-branch-line>` 的条目说明后出现的同名块会取代它、带 `primary` 的块就是主线移动时写下的那段说明，并且它归 TimelineTree——每段身份块都由它写——所以在任何执行配置下都在位。`<dim-branch-lines>` 的条目写明它的 key 是 `branch:roster`，所以读者不会把它当成 `branch:line` 下的身份。`<dim-branch-idle>` 和 `<dim-branch-ended>` 的条目归 BranchLine，两条通知都由它写；`<dim-branch-message>` 的条目与它们一起注册，因为 BranchLine 恰好在写消息的 BranchTool 所在之处加载。名册条目和消息条目在每条线上都在。

**策略。**

- **主线的协调文字。** `branch` 工具描述的指引：说明一条线是什么、以及对持有协调的那条线而言协调意味着什么的开头段、`create` 关于何时开线与 brief 带什么的规则、`message` 关于选模式和发决定而不是进度报告的规则，以及 `read` 关于读线上的工作而不是等报告的规则。[`BranchToolConfig.description`](../../../crates/dim-agent-capabilities/src/timeline_tree.rs) 只替换这段描述，不替换别的。`create` 规则里有两部分属于策略：开线之前必须全部成立的四道条件，以及与相邻机制按需求划定的分界（长命令在后台跑；不需要这段对话上下文的、或需要另一套指令的工作归另一个成员；一个大目标坚持做到底就在本线继续或用 goal；等会话之外的事情用事件订阅）。"When the user asks for a line, open it" 也是策略，它是四道条件之外的那条例外。`create` 规则的其余部分——`prompt` 承载什么、以及 `Returns the line's id`——是协议，见上。开头段里 "On the primary line you coordinate" 这一句及其三个从句是策略："answer a line's steer and act on what it asks for"、"leave a line's part to that line instead of redoing it" 和 "read a line's work instead of waiting for a report"。名册的收尾句 "They message you when they find something, need you, or end. Use the branch tool to message or read them, and the `wait` tool to wait for them; end the wait when your lines have nothing left to tell you." 也是协调文字，其中 "end the wait when your lines have nothing left to tell you" 覆盖结束开线方自己的等待；[`BranchToolConfig.roster_closing`](../../../crates/dim-agent-capabilities/src/timeline_tree.rs) 只替换这句，不替换别的。它覆盖 [§12.2](#122-交给提示词层的事项429) 的第一项：开线方怎样给它开的线写 brief、分配它的文件或区域（在 `create` 规则里）。
- **线的工作规则。** "How you work with P:" 的五条要点——目的、读取与消息（含 "Do not send P a progress report it can read" 和 "do not page through P's whole stream"）、工作区、用户和结束（含 "Read P once more before you end if the parent block says P has moved"）——属于策略，除了点明机制的从句："You cannot open lines unless you are the primary line"、动作名与模式名，以及 "the file was modified after it was last read"（`edit` 与 `write` 报告的那次失败），那些是协议，见上。它还没有工厂配置成员，在第一次为它评估候选的改动里获得成员。其中第三条要点的 "Change only the files P gave you" 与 `create` brief 里的文件区域是同一条规则，在这里写成线的这一面。
- **父线块的指引。** "Read it when what P did changes what you do, and before your final message; do not page through all of P's work."。它还没有工厂配置成员，在第一次为它评估候选的改动里获得成员。块的其余部分是协议，见上。
- **通知的指引。** "Read it if you need its state; message it with mode \"steer\" to give it more work, or leave it." 与 "read it if you need what it did."。它还没有工厂配置成员，在第一次为它评估候选的改动里获得成员。
- **主线状态说明的指引。** 七句属于策略，各自有自己的工厂配置成员：新主线上的 "The user switched here to continue directly from this line's context and approach: the user's request takes precedence over this purpose, which still says what this line is for."、"Carry this line's main work forward yourself; open a new line only when the four conditions for opening one hold again, not to hand off your current task." 与 "Avoid redoing work assigned to lines opened before your cut. ..."；用户开的新主线上，"Carry this line's work forward yourself; open a new line only when the four conditions for opening one hold again."；根上，"Carry the user's current work forward here; open a new line only when the four conditions for opening one hold again."；旧主线上，"Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line." 与 "You share the workspace with line N; do not edit files outside your remaining scope unless asked. ..."。`BranchToolConfig.primary_note` 与 `BranchToolConfig.former_primary_note` 替换它们，别的都不动；目的要点里的 "open one only when the four conditions ... and not to hand this purpose off" 是同一策略在说明里的读法。说明里其余全是协议：重述的身份、主线期间成立的规则、现在是哪条线是主线、旧主线继续开着并保留它的线、该线能否 `end` 及原因，以及 "If edit or write fails because the file was modified after it was last read"。

策略文本的候选放在该文本的工厂配置成员上（[面向模型的提示词 §8.1](model-prompts.md#81-规则)），并让它替换的文本里的协议句逐字节不变。

**这次改动的两道门禁。** 只有以下两者都成立才算接受这次提示词改动，而且只对这一改动有效：它们不改变 §8.1 的通用规则。第一，在[面向模型的提示词 §8.1](model-prompts.md#81-规则) 下每个场景都不退化：当前措辞对新措辞，在 20 次有效判定的下限上、以 [`xtask/src/rule.rs`](../../../xtask/src/rule.rs) 的 `DELTA` 0.05 与经 Holm 校正的 `ALPHA` 0.10，没有任何比较构成退化，并且每个场景都报告 `pass`。第二，在三个该开场景（[§12.3](#123-评估场景)）上开线率显著上升，且只按有效判定的运行比较。第二道门禁需要的运行次数高于下限：每臂 `n = 20` 时，从 0.10 升到 0.40 只有 36% 的检出率（[model-prompts.md:162](model-prompts.md)），所以在 80% 功效下三个比较本身就约需每臂 **44** 次，而规则一起判定的家族比三个大：每个（场景, 探针）组合、汇总的 `must_show` 与 `must_not_show` 行、失败运行率与未判定运行率。十五个场景共声明 36 个探针，家族再加上汇总的 `must_show`、汇总的 `must_not_show`、失败运行率与未判定运行率，因此 `m = 40`，比较在 α = 0.0025 下进行，80% 功效需要每臂 54 次；N 由计划按这个数定下，某个场景增删探针时重算。本文件只要求 N 至少为 44、按最终比较数用 Holm 重算，并且每个该开场景的每一臂都跑到 N。

这些策略文本作为同一家族一起判定，在第一次把它们放到位的那次改动里进行：工具描述在 `BranchToolConfig.description`，状态说明在 `BranchToolConfig.primary_note` 与 `former_primary_note`，名册收尾句在 `BranchToolConfig.roster_closing`。以后再改其中一段，要重跑整个家族，因为单独读起来顺的文本仍可能输掉另一段赢下的比较。

## 13. 相互影响

| 功能 | 影响 |
| --- | --- |
| Goal | Goal 自己的规则不变。一条线的开启输入按 A5 是有来源的输入（§7.4），Goal 的冲突表记录了这一点。新线照旧清除继承的目标。一条线读取它被切出来的那条线，所以评审者能读到执行者在切点那一轮之后的工作；Goal 的折叠不变，因为评审者是在自己线上通过工具结果读取的。Goal 的代码只随 Pursuit 修订改变：从 `info(自己的 id).source` 识别评审者、结束它的评审者，以及 `branch/ended` 这个 key（[Pursuit §13](plugins/Pursuit.md#13-结束评审者随-427)） |
| Pursuit | 评审者得到插件线的身份块，并且能读到执行者在它的切点之后的工作（§6）；它决定的是评审，判定方式不变（[Pursuit §13](plugins/Pursuit.md#13-结束评审者随-427)）。它是插件线，所以从不进名册、不成为主线、也不开线 |
| Fork、删除 | fork 照旧只拷根，所以 fork 出来的 Session 主线是根（[fork §6.2](appserver/session-deletion-and-fork.md#62-why-main-only)）。删除不变。已结束的线按它的事实处理（§10） |
| Session 关闭、空闲卸载 | 不算结束。各线恢复后仍未结束 |
| 中断 | 一次轮次中断，或对某条线的停止，只结束一轮，从不结束一条线。对主线的一次中断不影响它开过的线 |
| Session 的模型、profile、提示词、语言 | 模型、profile 和 `prompt.refresh` 作用于用户所在的那条线，而一条线的模型来自它继承的前缀，所以不是主线的线保持它自己的配置（§22）。语言是整个 Session 的，广播给每条已加载的线 |
| 定时任务 | 新建时没记录 `branch_id` 的任务，在创建那一刻采用当时的主线。已有任务保留它记录的 `branch_id`，所以移动主线从不改变已经排好的任务投到哪里 |
| Collaboration | 不变。成员是 Session；成员的 Session 是否挂载 `branch`，由它档案的工具列表决定 |

## 14. 客户端

**TUI。** 线有自己的面板、视图和操作。从当前 Timeline 的某一点开线或 fork、以及每条线的切点，见 [Timeline 优先的 TUI](timeline-ui.md)：

- `/lines` 列出这个 Session 的每条线，各自缩进在打开它的那条线之下，每条带状态：工作中、等待中、自行结束、被某个操作结束、随 X 结束，或创建失败。主线标作“主线”，根标作“起点”。
- Enter 显示所选线的对话。输入框向正在显示的线提交，输入框的下边框标出这条线，用 id 的前八个字符。
- `x` 再 `y` 结束所选线；根、主线以及主线的每一条祖先上被拒绝。
- `p` 把主线移到所选线上（“从这里继续”）；插件线、以及已经是主线的线上被拒绝，根在不是当前主线时同样是合法目标。面板写明旧主线照旧开着。
- 提问和审批都带线的标签，两条线问同一件事也能分辨（提问显示 `(branch NAME)`，审批显示 `Permission on branch NAME`）。
- 两条线之间的消息，在发送方读作已发出（它的 `branch` 工具调用），在接收方读作已收到（`◂ from C`）。
- 切换显示的线就在一条边界行（`line C`）之下换一份记录，和切换 Session 一样，因为终端的回滚缓冲只追加。发现分支时只保留任务目录和分支历史状态，不保留每条线的完整对话。首次从 `/lines` 或侧边视图打开一条线时，读取并保留它的[有限对话窗口](../../design/clients.md#opening-a-conversation-with-a-bounded-window)，此前发现失败也一样。窗口之外仍可操作的工作通过状态所有者接口恢复。选中线时另行请求显示详情。

**分支目录。** `session.branches {sessionId}`（v3，按与 `session.history` 相同的准入规则，在 `hello.features` 里与其他会话方法一起列出）为每条线回答一条记录：它的身份（`branchId`，有的话还有 `parentBranchId`、`branchPoint` 和 `createEvent`）、按 `TimelineInfo.source` 给出的 `source`、`state`（`open`、`ending` 或 `ended`）、`head`，处于结束中或已结束的线还有 `endEvent`。回复还带上一个顶层 `primaryBranchId`，即当前主线，而不是每行一个标记：顶层字段表达不出“零条或两条主线”这种非法状态，不认得这个字段的客户端会忽略它。`main` 排第一，其余按 `branchId` 顺序。回复不分页：它随线的数量增长，而线的数量由 Session 自己的使用决定，不随历史增长。答案归 `TimelineTree`（`directory()`）；`branches()` 列出尚未结束的线（`open` 和 `ending`），供持有活动线的调用方使用。

`session.branch_primary {sessionId, branchId}`（v3，在 `hello.features` 里与其他会话方法一起列出）移动主线。目标是开着的对话线：工具线、用户线，以及根——根只要不是当前主线就是合法目标，于是用户能把主线移回这个 Session 的起点。插件线永远不是目标。除此之外只拒绝一种目标：已经是主线的线，不管那条线是哪条，拒绝码 `branch_already_primary`；其余情况下它在根上写 `branch/primary`，并写两条状态说明（§12.1）。它回新的 `primaryBranchId`。这就是“从这里继续”这个动作：不写任何 Session 级配置，也不碰其他线。

目录读取不访问历史，显示字段为空或省略。`session.branch.get {sessionId, branchId}`（v3，在 `hello.features` 单独声明）读取一条线的目录记录和显示详情：`purpose` 为创建提示的首行；`cutMessages` 为该分叉点继承的用户输入数，排除截止位置前已删除的输入；`end` 为可用的 `BranchEndData`。根的前两项为空，没有可读结束事实时 `end` 为空。该读取由 TimelineTree 完成点查与固定前缀计数；未知分支和读取失败在本次详情调用中报错，不阻断目录列举，不写事实，也不重新打开结束的线。CLI 在选中一条线时才读取详情。


`head` 有三条规则，决定客户端能用它做什么：

- 已结束（`ended`）线的 `head` 和 `endEvent` 固定。客户端把 `head` 作为 `through` 传给 `session.history` 读这条线；`endEvent` 是这条线自己的 `branch/end` 序号，祖先结束级联到它时也一样。
- 结束中（`ending`）线的 `endEvent` 是它已提交的 `branch/end`，而 `head` 仍是活值：这条线的已结束记录以及 `branch/ended` 都还在后面。一条线从它自己的 `branch/end` 提交起——工具结束、host 结束或级联结束——一直读作 `ending`，直到它的已结束记录被写入，期间输入一直被拒绝。运行中的轮次里由工具发起的结束，会一直保持 `ending`，直到那一轮结束、关闭完成。重启不保留这个窗口：`branch/end` 已提交、但已结束记录从未写入的线，恢复后是 `ended`（§10）。
- 未结束（`open`）线的 `head` 是活值。恢复会给每条未结束线追加 `branch/resume` 和中断事实，因此重启后 `head` 会前进；客户端在重连后重新读目录，不要缓存 head。

这个面板与任务面板分开：任务面板列后台命令、成员和 Workflow 运行，它的 `x` 是停止任务，而结束是永久的，两者不共用按键。

```
┌ session 3f2a · 主线 b-7c ────────────────────────────────────────┐
│ … b-7c 的对话 …                                                   │
│ ◂ from main (steer)  the leak is in cache.rs                    │
│ ◂ from b-02 (steer), final  docs pass done                      │
│                                                                 │
│ ╭─────────────────────────────────────────────────────────────╮ │
│ │ › ask anything…                                             │ │
│ ╰───────────── ~/repo/dim (main) ────────────────────────────╯ │
└─────────────────────────────────────────────────────────────────┘

┌ lines ──────────────────────────────────────────────────────────┐
│ ○ main             起点                                         │
│   b-7c  parser race hypothesis      working · 主线 · shown      │
│   b-91  review the cache change     waiting                     │
│     b-c3  the guard in cache.rs     working                     │
│   b-02  docs pass                   ended itself                │
└ Enter 显示 · p 从这里继续 · x 结束 · Esc 关闭 ───────────────────┘
```

**Web** 把主线居中，在右侧详情面板里跟随其余线：Lines 树、每个打开的线一个标签，以及按 line 的发送、停止、审批和结束（[Web 工作区 §4.1](../../design/web-workspace.md#41-lines-and-branch)）。移动主线的动作与 TUI 相同，经 `session.branch_primary`。会话列表里的运行状态和最近活动来自主线，所以对话在一条线上时，这个 Session 由那条线表现为在跑或最近活跃，而不是由根。Desktop 得到的是协议：`session.branch_end`、`session.branch_primary`，以及 v3 历史里的新事件，加上目录上的 `primaryBranchId`。

## 15. 改动

| 模块 | 改动 | 原因 |
| --- | --- | --- |
| [设计规则 §2](../../architecture/rust-agent/principles.md#2-作用域的含义) | 改写：同一主体的多条线、共享事实、父子是唯一的通信和读取关系、共用交互面。关系表一行改为"只在父子之间"，读取父线已提交的事件经工具结果；Session 一行加上"结束" | 现在的文本是"平等的线，父子只记录来源"，没有通信关系 |
| TimelineTree | 对每种来源都在 `first_input` 里写身份块。新增 `TreeCommands.end(id, source)` 和 `TreeCommands.page(id, after, limit)`。`TimelineInfo.source`，依据父分支的 `branch/create` 填写（§5）。结束流程及其守卫、线进入结束后拒绝输入、`branch/ended`、恢复规则、读已结束的线，以及主线：根上的 `branch/primary`、它的恢复读取，和一次移动写下的事实。它的 `directory()` 报告未结束、结束中和已结束的线：结束中的线从工具、用户或级联写入它自己的 `branch/end` 起，到它的已结束记录被写入为止，带该事件与活头部，并带上当前 `primaryBranchId`。它的节点把这条线最新轮次结束的序号发布给它开出的线（§7.1.1） | 它拥有线的生命周期、开启输入，以及主线唯一那条权威事实 |
| TreeTimeline | 消费 `branch/end`，以及由工具调用发出的结束时这条线的下一次轮次结束，并转给 TimelineTree。它把每一次 `turn/end` 报告给 TimelineTree，后者为这条线的子线发布 | 它现在就这样转 `branch/create` |
| BranchTool | 按动作区分的工具，加上新的 BranchLine 分支部件。`create` 受两道守卫：调用方是当前主线，以及这个 Session 的工具开线上限。`read` 和 `message` 既能读、写调用方被切出来的那条线，也能读写它开过的线，共用一条游标规则 | 工具的各个动作，以及一条线对自己相关各线的视图 |
| Session 门面 | `Session::end_branch(id, source)`、`Session::primary(id)` 与 `Session::set_primary(id, source)`。已结束线的头部由 `Session::sequence` 给出；其历史在固定的头部经 `TreeCommands.page` 读取 | §17 |
| App Server | `session.branch_end`（v3，用户的结束，带生成的 `operation_id`；`session_execution` 打开、且连接的入口不是裸入口或守护进程 profile 未受保护时，`hello.features` 把它和其他会话方法一起列出）。`session.branch_primary`（v3，同样的准入）。对已结束的线，`session.history` 立即返回，截到它存储的头部。`session.branches` 读树的目录并带上 `primaryBranchId`（§14）。今天写成 `main` 的每一处 Session 级缺省目标改读主线：`session.submit`、`session.stop`、`session.idle`、`approval.respond`、`session.goal.*`、`shell.run`、会话目录的运行状态与最近活动，以及新建的定时任务。`session.model.update`、`session.agent.update` 和 `session.prompt.refresh` 作用于请求指明的那条线，而不再拒绝根以外的每条线。`ProductEvent::new` 在 v3 里带上新的事件种类，v1、v2 里不带 | 两个入口和三条读取路径，没有业务逻辑 |
| dim-agent-types | 在 `EventKind` 和 v1 排除列表里加入 `branch/end` 和 `branch/ended`，随后加入 `branch/primary`，并在契约测试中登记。`BranchSource` 新增用户开线的种类，与插件区开。冻结样本不变 | 核心事实（§16） |
| 存储 | `FORMAT` 52 → 53，一条空标记升级 `(52, &[])`：不扫表、不迁移数据，于是旧版 daemon 干净地拒绝这个库，而不是在第一次解码时报错。`SqliteTimelineStore` 的写线程为目录的“最近活动”缓存每个 Session 的主线，用恢复所用的同一条索引点查初始化 | 新的事件种类和新的 `BranchSource` 变体是 52 版解不出来的行 |
| Goal（Pursuit 修订） | 从线的来源识别它的评审者，并结束它（[Pursuit §13](plugins/Pursuit.md#13-结束评审者随-427)） | 评审者的协议归 Goal |
| 分支 TypeScript 绑定 | 重新生成 `npm/branch-services.d.ts`：TreeCommands 新增 `end` 和 `page`，`TimelineInfo` 新增 `source`，`LineRole` 指的从协调者改为根，目录带上 `primaryBranchId`。[绑定表](../../design/branch-typescript.md)中 `treeCommands` 一行列出 `end` | TreeCommands 是导出的服务 |
| CLI TUI | §14 | 用户跟随线、与线对话、把主线移过去、结束线 |
| 插件页面 | TimelineTree.md 和 BranchTool.md 随实现更新（`xtask check`） | 页面跟随代码 |

新增的公开内容：

- 三种事件：`branch/end`、`branch/ended` 和 `branch/primary`；
- 四个 RPC 方法：`session.branch_end`、`session.branch_primary`、`session.branches` 和 `session.branch.get`；
- 两个 TreeCommands 方法：`end(id, source)` 和 `page(id, after, limit)`，以及主线的读与写；
- 目录回复上的 `primaryBranchId`；
- `TimelineInfo.source`，由 BranchLine 和 Goal 读取；
- `BranchSource` 里用户开线的种类；
- `branch` 工具的新输入。

BranchLine 的服务只由同一插件里的工具读取。轮次结束信号是 crate 内部的，与 `TreeCommands.page`、`head` 并列：它不导出，TypeScript 绑定不变。

## 16. 格式

`branch/end` 和 `branch/ended` 是 `branch` 领域的核心事件，`branch/primary` 也属于这个领域：

| 事件 | 数据 |
| --- | --- |
| `branch/end` | `{source, ancestor?}`。`source` 是 `BranchSource`（`{kind: "tool", turn_id, request_id, call_id}`、`{kind: "user", operation_id}` 或 `{kind: "host", operation_id}`）；`ancestor` 是一条线的 id |
| `branch/ended` | `{branch, endEvent, source, ancestor?}`。`endEvent` 是 `branch/end` 在结束的那条线上的序号；`source` 与 `ancestor` 从它复制 |
| `branch/primary` | `{branch, source}`。只写在根上。`branch` 是这个 Session 的对话所在的线；`source` 是移动它的那次操作。根上最新的一条就是主线，除非那条线上之后又写了一条 `session/forked`，把它作废 |

它们的格式决定记录在存储合同里：[#427 v20 格式决定](../../design/appserver/native-storage-format.md#427-v20-格式决定)。`branch/primary` 改变了已存行的形状，所以存储的 `FORMAT` 随之升高（§15）。

## 17. 向机制所有者提出的需求

**R1. 一条线的持久结束**（所有者：dim 运行时，即 TimelineTree 与 Session 门面；随本设计在 #427 下实现）。

- **被挡住的功能与规则。**
  - 一条线无法单独结束。`Branch::close()` 存在，但没有任何入口调用它，关闭也不留记录，所以恢复时这条线又被打开。
  - Session 门面只从已发布的线读历史，所以已结束线的历史读不到了。
  - 门面和事件格式属于基础设施（[设计规则 §4](../../architecture/rust-agent/principles.md#4-通过时间线扩展行为) 第 8 条）。
- **考虑过的现有机制。**
  - `stop/interrupt` 结束的是一轮，不是一条线。
  - Session 关闭会结束所有线。
  - `branch/create-failed` 只在创建完成之前存在。
  - 插件自有的事件行不通，因为恢复归 TimelineTree，它不能读插件的事实（第 4、7 条）。
  - 由 App Server 直接读存储会绕过门面。
- **通用能力。** 不带业务含义：
  - 线上的指令 `branch/end {source, ancestor?}`，以及父分支上的结果 `branch/ended {branch, endEvent, source, ancestor?}`；
  - `TreeCommands.end(id, source)` 和 `Session::end_branch(id, source)`，它们只检查目标；
  - `TimelineInfo.source`，依据父分支的 `branch/create` 填写；
  - `branch/end` 提交后不再接受新输入；
  - 先结束后代；
  - 恢复时把已结束的线视为不活跃，并为没有 `branch/ended` 的 `branch/end` 补完结束；
  - 已结束线已提交的历史在固定的头部仍可读取，经 `TreeCommands.page`。
- **对序号、批次、中断、重载的影响。**
  - 每个事件单独一个批次：线上的 `branch/end`，以及关闭之后父分支上的 `branch/ended`。
  - `host` 来源的结束，或因祖先而结束，经 `Branch::close()` 中断这条线。
  - 重载时适用上面的恢复规则。Session 在恢复完成后才发布，所以在加载时结束一条线的部件，例如 Goal 的核对（[Pursuit §13](plugins/Pursuit.md#13-结束评审者随-427)），要等 `SessionLifecycle::activated()`，此后它看到的每条线（目录里有的）都已发布。
  - Timeline 多一道线自己的准入检查：`Timeline::input` 变为 `input_checked(data, admit)`，在提交锁内运行 `admit`，于是 `branch/end` 之后提交的输入以 `branch_ended` 被拒绝，而不是与关闭竞争。Dispatcher 和批次都不变。

## 18. 安全

- 各线共用 Session 的权限策略和 Approval。一条线不是隔离手段（[ADR 003](../adr/003-approval-scope.md)）。
- 只有 TimelineTree 和 BranchLine 写这些标记。其他写块的生产者（经 `TreeCommands.input` 的插件）是受信任的进程内部件。RPC 不写块。
- `session.branch_end` 和 `session.branch_primary` 与 `session.branch_create` 一样，是受信任的用户入口。它们不需要审批，因为它们不是 agent 的动作。
- 任何受信任的进程内部件都能结束结束守卫允许的任一条未结束的线，就像它能向任一条线写输入。工具的 `end` 与其他调用一样受 Approval 约束。
- 主线不带任何权限。移动它改变的是缺省目标解析到哪条线、以及哪条线能开线；它不扩大任何一条线的 Permission、Approval 或文件根（[ADR 003](../adr/003-approval-scope.md)）。
- 诊断只记录 id、序号和来源的种类，从不记录消息或目的的文本。

## 19. 规则检查

规则：[设计规则 §1、§2 与 §4](../../architecture/rust-agent/principles.md)、[ADR 002](../adr/002-plugin-scope.md) D1–D5、[ADR 003](../adr/003-approval-scope.md)。

| 规则 | 本设计如何遵守 |
| --- | --- |
| §1 Capability 与 App Server 边界 | Capability：TreeTimeline（已有）和 BranchLine（新）。TimelineTree 和 BranchTool 的 Session 部件是普通部件。App Server 只增加两个入口和一条读取路径，没有业务代码 |
| §2 作用域 | 线的生命周期仍归 Session（TimelineTree）。一条线对自己相关各线的视图是分支状态（BranchLine）。线之间只靠写输入互相影响，只靠读取了解对方，现在只限父子之间（§2 已改写）。能力影响分支不受这条规则约束（§4） |
| §4.1 指令事件 | 结束是 `branch/end`，由 TimelineTree 决定并写 `branch/ended`。消息是输入，由接收方的 InputQueue 和 Loop 决定。结束一条已结束的线、或被拒绝的消息，什么也不写，回复说明原因 |
| §4.2 交接后不撤回 | 已提交的消息属于接收方。`branch/end` 不撤回 |
| §4.3 before 回调 | 无 |
| §4.4 核心不带业务含义 | Dispatcher、`before` 和批次不变；Timeline 的准入检查是通用的（§17 R1）。身份文本属于 TimelineTree 自己的领域，即一个 Session 的各条线。工具线的协议写在 `first_input` 现在写工具专用文本的地方，也就是拥有该工具的模块里 |
| §4.5 批次 | `branch/end` 单独一个批次。`branch/ended` 在关闭之后最后写入。名册和结束通知是同一条输入。根上的 `branch/primary` 单独一个批次，两条状态说明各自在它之后一条 Append |
| §4.6 上下文 | 身份、名册、消息、通知和主线状态说明都是被采用的输入。`read` 返回工具结果。没有别的东西进入上下文 |
| §4.7 公开事件与服务 | BranchLine 折叠的是它自己那条线的公开事件。工具读 BranchLine（同一插件）和 TreeCommands。`read` 像 Collaboration 的 `read` 一样，把已提交的事件作为工具结果返回，没有哪个部件折叠另一条线的事实。Goal 按它自己公开的 A5 约定判定开启输入（§7.4）。BranchLine 和 Goal 经公开服务 TreeCommands 读取 `TimelineInfo.source` |
| §4.8 基础设施 | 门面和格式的改动即 R1（§17）。需求来自线的生命周期本身，在运行时内闭环 |
| §4.9 只有后续决定要用的事实才进 Timeline | `branch/end` 决定关闭和恢复。`branch/ended` 决定恢复时是否活跃，以及名册。`branch/primary` 决定这个 Session 的缺省目标解析到哪条线、以及哪条线能开线；它只携带一个角色和移动它的那次操作，不携带别的。没有只给 UI 用的事件，也没有只为展示而设的字段 |
| ADR 002 D1 作用域 | 见 §5 |
| D2 不设接线模块 | BranchTool 的 feature 自己登记 BranchLine |
| D3 作用域依赖 | BranchLine 向外依赖 Session 的 TreeCommands。没有核心部件依赖它 |
| D4 配置 | 无 |
| D5 合同语言中立 | 事件、RPC 方法、标记和工具 schema 都是数据。TreeCommands 的新方法已导出，并为 TypeScript 生成类型 |
| ADR 003 审批 | 工具风险仍是 `session`，Approval 放行每次调用。用户结束一条线不是 agent 的动作 |

## 20. 验收

以下是使用假模型的流程，放在 `branch_flow.rs`、`flows.rs` 和 App Server 传输流程中。

1. 主线开两条线，名册列出这两条。每个子分支的身份（`branch:line`）不碰它继承的名册，自己又开过线的子分支两者都保留。开线方和某个子分支各自压缩后，带 key 的基线分别重放每份名册和每份身份。
2. 子分支给它的开线方发一条 Append：它不唤醒开线方，开线方在下一次请求时看到。Steer 唤醒开线方的 `wait`，或在开线方空闲时开启新的一轮。来自开线方的 Steer 唤醒空闲的子分支。
3. 下列操作被拒绝，什么也不写：给兄弟分支发消息或读取、一条线和它的祖父级、在不当前主线的线上 `create`、对插件线 `message`、读插件线、在根上、主线上或主线的祖先上 `end`。
4. 子分支的一轮没有发 steer 就结束时，开它的那条线收到一条带这一轮状态的空闲通知。这一轮 steer 过它的开线方、调用了 `end`、留下了待处理的驱动输入，或者这条线就是当前主线时，不写通知。
5. 工具线结束自己：写入最终消息（steer）和带 `tool` 来源的 `branch/end`，在这一轮结束时关闭，然后在它的开线方上写 `branch/ended` 并更新名册，不写结束通知。之后的消息被拒绝（`branch_ended`）。工具的 `read` 和 `session.history` 都能返回已结束线的历史。
6. 经 `session.branch_end` 由用户结束：`branch/end` 带 App Server id 的用户来源，这条线当前这一轮被中断，它的开线方以 steer 收到结束通知。根、主线、以及主线的祖先都不能被结束。再次结束返回 `branch_ended`。向已结束的线提交返回 `branch_ended`。
7. 恢复：已结束的线不打开，它的后代也不打开。没有 `branch/ended` 的 `branch/end` 在重启时补写结果。
8. Pursuit 的评审者得到插件线的身份块，能读到执行者在它的切点之后的工作，现有的 Pursuit 流程照常通过。
9. 两条线编辑同一个文件：后编辑的那条得到 `file_modified_since_read`（已有行为）。
10. v1、v2 客户端看不到新事件。v3 历史包含它们，`hello.features` 列出 `session.branch_end` 和 `session.branch_primary`。
11. 读取父线：一条线第一次读它被切出来的那条线时，从切点之后开始；低于切点的显式 `after` 会抬到切点；高于目标头部的显式 `after` 由工具结果拒绝并报出该头部，不存任何东西；不带 `after` 的读取从调用方上一次的 `nextCursor` 继续，两个方向都一样，并且重载后仍然保留；对它开过的线给 `after: 0` 会从头重读。
12. 父线块：空闲的线被告知父线动了而不会被开启一轮，块在它下一轮的第一个请求里、先于开启那一轮的输入；运行中的线在下一次模型请求时拿到它，且只拿一次；被持有的块不会被覆盖；被消费的块之后，新的头部带来新的块；已经读到头部的线不会被通知；只为开启这条线的那一轮不算活动，而之后的工具调用或操作者输入算；没有移动的重载不写任何东西，有到期块的重载写一个；结束中的线和根都拿不到；对这条线的一次压缩之后块会重放；嵌套线的开启输入带着名字它自己父线、切点为头部的那个块，而从根切出来的线拿不到。
13. 父线块在评审者线上不改变任何 Goal 决定：它是一条 Append 输入，第一个块不是 goal 标记，key 也不是 goal key，所以折叠的 `driving`、`lastOperator`、计数和结论都不变（`goal.test.mjs`）。

Goal 流程，放在 `crates/dim-goal/tests` 的流程测试中（§7.4）：

11. 用 `session.branch_create` 开出的线，开启输入带身份块，表现与根开出的线完全相同：在这条线上 `set`、它的续跑，以及之后一次重置计数的操作者输入。
12. 在新的开启输入下创建的 Pursuit 评审者能被识别，它的结论会被应用。工作线的计数不变。
13. 身份块不改变目标和状态折叠、已决定集合或续跑列表中的任何内容。

线的结束与来源：

14. `TreeCommands.end` 结束结束守卫允许的任一条未结束的线，不管是谁调用。例如插件部件以 `host` 来源结束一条工具线，它的开线方就收到结束通知。它拒绝根、当前主线、主线的祖先、未知的线和已结束的线，什么也不写。
15. 结束一条有子分支的插件线时，先结束子分支（带 `ancestor`，复制 `source`），再结束这条线。若一条线的后代里有当前主线，这次结束在写入任何东西之前就被拒绝。
16. `TreeCommands.list` 和 `info` 对新建和恢复的子分支返回 `source`，对根不返回。存储的目录行逐字节不变。

分支目录：

17. `session.branches` 先返回 `main`，其余按 `branchId` 顺序，每条带身份、来源、状态和 head，外加一个顶层 `primaryBranchId`。级联结束时，被结束的子分支以它自己的 `branch/end` 序号作为 `endEvent` 记入目录；把该子分支的 `head` 作为 `through` 传给 `session.history` 能读到它。一条线从它自己的 `branch/end` 提交起——工具结束、host 结束或级联结束——读作 `ending`，带该事件和活头部，直到它的已结束记录被写入；之后读作 `ended`，事件不变、头部固定。运行中的轮次里由工具发起的结束，会一直保持 `ending`，直到那一轮结束、关闭完成。
18. 重启后目录保持 id、父分支、切点、创建事件和状态；已结束线保持 `head` 和 `endEvent`，未结束线的 `head` 不低于旧值。在一轮仍在运行时结束的线，恢复后是 `ended`，不是 `ending`。
19. fork 的目录列出 fork 之前已结束的线，带来源记录的结束，而那些线在 fork 里没有事件。v1、v2 客户端没有 `session.branches`；已关闭的 Session 回答 `session_closed`。

主线：

20. 只一条线的 Session 就在根上，这次恢复不为它查任何东西。有多条线的 Session 在根上只读一条事实，回答最新一条 `branch/primary` 的目标；两类里最新的是 `session/forked` 时，或者什么也没时，回答根。目标已结束、或者目标是插件线时，回答根。
21. `session.branch_primary` 移动主线：`branch/primary` 写在根上，恰好写两条状态说明（新主线和旧主线），其他每条线什么也收不到。拒绝时什么也不写：插件线、已结束的线、未知的线，以及已经是主线的那条线，后者取 `branch_already_primary`，所以根在主线上时被拒绝、在别的线当主线时作为目标被接受。
22. 移动之后，`session.submit`、`session.stop`、`session.idle`、`approval.respond`、`session.goal.*`、`shell.run`、目录的运行状态与最近活动、以及新建的定时任务，都解析到新主线。已记录 `branch_id` 的定时任务保留它。
23. 旧主线仍能给它开过的线发消息、读它们、结束它们；不是主线的线 `create` 被 `branch_not_primary` 拒绝。原来是工具线而现在成为主线的线停发空闲通知，对自己 `end` 被拒绝。
24. 主线不在根上的 Session 被 fork 后，fork 出来的会话的主线是根，因为拷进来的每一条 `branch/primary` 都比它的 `session/forked` 旧。
25. 这个 Session 把工具开着的线开到上限后，再开线被 `branch_limit_reached` 拒绝；上限按全树统计所有仍开着的工具线；客户端开的线和插件线从不计入，也不被上限拒绝。

## 21. 消融审查

留下的部分，以及每一部分为什么必要：

| 部分 | 原因 |
| --- | --- |
| 每个子分支上的身份块，同一个 key | 没有它，一条线压缩后会忘记自己是哪条线，或把开线方的名册当成自己的 |
| 名册有自己的 key，写在每个开过线的线上 | 协调沿父子关系走，所以每个开线方都需要名册；与身份共用一个 key 会让写名册把非根线自己的身份退掉 |
| 消息是 `input/message`，带 append 和 steer | 这是线之间唯一的通道。模式是 InputQueue 的 |
| 开线方的 `read` | 开线方不用开口问，就能检查一条线的工作 |
| 空闲通知 | 没有它，开线方会一直等一条悄悄停下的线 |
| `branch/end` 与 `branch/ended` | 没有它们，一条线无法持久结束，恢复时又会被打开 |
| 结束守卫 | 结束一条线会连带结束它下面的每一条，所以必须拒绝主线和它的整条链，否则一次祖先的结束会把刚做完的移动撤销 |
| 结束上的 `source`（`BranchSource`） | 创建和结束以同样的方式记录发起方，插件的结束不会被记成这条线自己的结束 |
| 用户开线的来源种类 | 只有对话线能当主线、能开线，而核心不能靠解析插件的 `operation_id` 来分辨两者 |
| `TimelineInfo.source` | 开线的一方不用解析 prompt 文本就能找到自己的线 |
| 根上的 `branch/primary` | 一条唯一权威、非模型的事实：App Server、存储写线程和恢复都读它，不经过模型或键控块，而根又不能结束 |
| 两条状态说明 | 规则变了的只有两条线，各得知一次；其它线一概不碰，所以其它线的缓存前缀不移动 |
| 开着的工具线上限 | 树只在用户移动主线时才会变深，总数能限住工作量，又不用一个会禁掉第三次合理移动的深度上限 |
| 目录里的 `primaryBranchId`、`endEvent` 与 `ending` 状态 | 没有 `primaryBranchId`，客户端居中不到正确的线；没有 `endEvent`，客户端无法指出级联结束那条线的结束；没有 `ending`，从一条线自己的 `branch/end` 到它的已结束记录之间的窗口会读作未结束 |
| 工具限于当前主线、以及调用方开过的线 | 一条线承载会话对话、能长树；创建者保有各自协议，协调留在它被建立的地方 |
| 关于文件的提示词规则 | 唯一的保护是 FileReads，其余靠协议 |

删掉的：

- 对消息数、steer 数或往复轮数的上限；
- 文件改动的归属；
- 对 WorkspaceTools 的改动；
- 兄弟分支之间的通道；
- 按线的配置（system、模型、工具）；
- 结果约定与结果回交；
- 为移动主线单独设的事件或概念（它就是一条 `branch/primary` 事实）；
- 为消息单独设的事件或概念（它们就是 InputQueue 的模式）；
- 开线方经工具结束它开的线；
- 取值固定的 `by` 字段（现有的 `BranchSource` 记录发起方）；
- 插件线上的分类字段（`operation_id` 已经是开线一方的标签）；
- 运行时检查谁在结束一条线；
- 与总数上限分开的深度上限（深度只在用户移动时增长）；
- 已存的 `primary_branch_id` 列（事实是一条事件，写线程缓存它）。

## 22. 以后的选项

**在 Session 的限度内按线收窄。** 在 Session 的限度内收窄仍是同一个主体，所以与设计规则 §2 相容。它不属于本设计。做的时候：

- **更少的工具：** 在调用时拒绝，就像现在不是主线的线上 `create` 被拒绝那样。工具定义和缓存前缀保持一致。
- **更低的权限：** 每条线一份策略，与 Session 的策略取交集。它需要 [Pursuit §10](plugins/Pursuit.md#10-以后的选项) 已经提到的按分支收窄 Permission。
- **更便宜的模型：** 在创建时、第一条输入之前设定的按线配置。不同的模型与父分支不共享缓存，所以它是用继承的前缀换成本。

**按线的模型、profile 与提示词。** 模型、profile 和 `prompt.refresh` 作用于请求指明的那条线，而一条线自己的配置来自它继承的前缀。线保自己历史里的那份配置，所以移动主线不改变新主线早先那些轮次是按什么配置跑的；改动作用于用户所在的那条线。语言是整个 Session 的，广播给每条已加载的线。
