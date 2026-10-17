# Recall

状态：草案。Issue：[#129](https://github.com/arcships/dim/issues/129)。里程碑：M2。语言：Rust。作用域：[ADR 002](../../adr/002-plugin-scope.md)（Session）。依赖 [ContextCompaction](ContextCompaction.md)（F1，已实现）与 `toolContribution` 挂接规则（C3，已实现）。

Recall 是一个 Session 插件，给模型提供一个只读工具 `recall`。它把模型所在分支的历史讲给模型听，讲的是经历：用户说了什么，模型说了什么、做了什么、得到了什么，以及期间还发生了什么。它经 Branch 门面读取已发布的 Timeline 事件，并把有界的文本作为工具结果返回。它不写事件、不持有状态，也不改动任何其他模块或类型。

## 1. 问题与范围

压缩把早期对话替换为摘要（[ContextCompaction §4.3](ContextCompaction.md#43-事件)）。摘要只保留转述；用户的原话和示例、之前抓取的内容，都从模型的视野中消失了。Timeline 仍保存着每一个事件。

Recall 不是"切点之后"的视图，而是对整个分支历史（含继承前缀）的回忆，就像人回忆一天的经历：压缩是其中发生过的一件事，不是边界。

**范围外：** 其他分支和 Session、原始事件数据，以及对压缩、摘要或上下文拼装的任何改动。

## 2. 模型怎么回忆

Recall 就像客户端里的会话视图，只是讲给模型而不是展示给用户。它分三层：

1. **轮次。** 每轮一行：什么时候、用户要什么、模型做了什么、这一轮怎么结束。从最近一轮往前分页。
2. **一轮。** 按顺序讲这一轮：发生的每件事一行，各带时间和 id。
3. **一件事。** 按 id 给出全文。

检索（`about`、`tool`）从所有轮次中返回命中的第 2 层的行，id 相同。每个结果最后都给出下一步的参数，所以第一次调用不需要任何先验知识。

```
 recall()                         recall(open="e41c07d2")              recall(open="3f9a2c1b")
 +----------------------------+   +-------------------------------+    +-------------------------+
 | Turn 14:21 e41c07d2        |   | 14:03 9b1e4d70 user: Use the  |    | Rate limits: 600        |
 |   user: "Stop. Don't …"    |-->|   staging bucket, never prod… |--->| requests per minute per |
 | Turn 14:03 c2d8a915        |   | 14:04 3f9a2c1b you called     |    | key; bursts of 50 …     |
 |   user: "Use the staging…" |   |   web_fetch → ok, "Rate lim…" |    | End.                    |
 +----------------------------+   +-------------------------------+    +-------------------------+
```

## 3. 叙事投影

`project_history` 讲不了这个故事：它是为拼装模型请求服务的，把一次调用拆成一个 `ToolUse` 部分和一条单独的 `ToolResult` 消息，其余什么都不保留。审批、后台执行、子 agent、从未执行的调用都不在其中。所以 Recall 在事件之上有自己的投影。它用与上下文拼装相同的规则判断什么算数（§3.3），再以自己的形式讲出来。

### 3.1 固定的读取

每次调用开始时，工具读取 `head = branch.sequence()`（已发布序号）和 `events = branch.history(Some(head))`，即分支的继承前缀加上其自身事件。其余一切都基于这一份固定的列表计算，所以一次调用不会混用两个 head。这与 `wait` 已有的做法一致（[wait.rs](../../../../crates/dim-agent-capabilities/src/wait.rs)）：一个 Session 工具从 `ToolExecutionContext.branch_id` 经 `TreeCommands` 找到调用方分支。Recall 不应用压缩，遍历的是整个列表。

### 3.2 事：一个起点事件和挂到它上面的事件

投影把事件折叠成**事**。一件事有一个**起点事件**，以及之后通过事件本来就带着的 id 挂到它上面的事件。一件事的那一行显示起点的时间和 id；它的结局是折叠完所有挂上来的事件之后剩下的状态。

| 事 | 起点事件 | 挂上来的事件（关联键） |
| --- | --- | --- |
| 用户输入 | 带用户内容的 `input/message` | `input/consumed`（`input_id`） |
| 运行时块 | `input/message` 中的一个块 | `input/consumed`（`input_id`）；之后同一 `key` 的块 |
| 你的回复 | `step/model-content` | `step/model-output-end`（`request_id`） |
| 工具调用 | `step/tool-use` | 来源为 `tool` 的 `approval/request`（`call_id`），然后是 `approval/respond` 或 `approval/cancelled`（`approval_id`）；`step/tool-result` 中对应的那一项（`call_id`）；`exec/started`（`call_id`）与 `exec/ended`（`task_id`）；`collaboration/request`（`call_id`），然后是 `collaboration/message`（`call_id`）与 `collaboration/result`（`request_event_id`）；来源为 `tool` 的 `branch/create`（`call_id`） |
| 中断 | `stop/interrupt` | — |
| 权限变更 | `permission/changed` | — |
| 压缩 | 结果为 `completed` 的 `context/compaction` | — |
| fork | 本分支的 `branch/create` | — |

后四种从未进入模型上下文，但它们属于发生过的事：用户的决定，以及对话的形状。

其余事件都是机制。它们作为中间状态被折叠或被丢弃，从不单独讲：`model/*`、`context/prepare`、`context/budget`、`context/configured`、`context/compact`、`context/compaction-start`、流式的 `step/*` 片段、`step/tool-call`、`step/tool-start`、`step/tool-completed`、`prompt/configure-*`、`input/applied`、`input/withdraw`、`input/deleted`。`turn/start` 与 `turn/end` 划分轮次（§3.6）。

### 3.3 什么算数

投影用上下文拼装的规则判断什么算数，所以"模型经历过什么"只有一个定义：

- 输入在被消费后才算；待处理、已撤回或已删除的输入不是一件事；
- 回复在其输出完成时才算，或者作为中断或失败留下的部分文本；
- 工具结果按模型收到的样子讲：它的 `content`，而不是 `structuredContent`。

一个流程测试把两种投影绑在一起：`project_history` 放进上下文的每个事件，都是某件事的起点或挂在某件事上（§14）。

### 3.4 每件事怎么讲

| 事 | 行 |
| --- | --- |
| 用户输入 | `user: <文本>`，原文，图片为 `[image]`；`steer` 输入为 `user (while you were working): <文本>` |
| 运行时块 | 按其标签和 `source` 属性，而不是外壳：`rule <source> set|replaced|revoked`、`state <source> updated|cleared`，其他为 `<tag>: <第一行>` |
| 你的回复 | `you: <文本>`，省略 thinking；未完成的输出为 `your reply was cut off (<原因>): <部分文本>` |
| 工具调用 | `you called <tool> <参数简述> → <结局>` |
| 中断 | `the user interrupted the turn` |
| 权限变更 | `permission mode changed to <mode>` |
| 压缩 | `the conversation up to here was summarized` |
| fork | `this branch was forked from <branch>; its task: <prompt>` |

Recall 按标签、`source` 和 `key` 通用地读取运行时块；它不认识任何生产者，所以新增一种块不需要改它。

**工具调用的结局**是折叠后的状态，取第一个适用的：

| 挂上来的事件 | 结局 |
| --- | --- |
| `approval/respond` 为 deny | `denied by the user` |
| `approval/cancelled` | `approval cancelled: <reason>` |
| 关闭一个从未派发的调用的结果（没有 `call_event_id`） | `not run: <结果文本>` |
| `collaboration/result` | `sub-agent <agent> answered: <文本>`（或其失败） |
| `exec/ended` | `ran in background task <task>; finished at HH:MM: ok|error|cancelled: <文本>` |
| 有 `exec/started` 但没有 `exec/ended` | `running in background task <task>` |
| 带 `is_error` 的结果 | `error: <文本>` |
| 结果 | `ok: <文本>` |
| 都没有（head 停在调用中间） | `no result yet` |

**参数简述**对所有工具通用：`input` 的标量字段写作 `key=value`，字符串截到约 60 个字符，嵌套的值显示为 `{…}`。展开这次调用时给出完整参数。

**结果**是 `content` 的文本；blob 和图片为 `[file]` 与 `[image]`；工具带入的上下文消息为 `with <n> context messages`。展开这次调用时给出全部内容。结果比调用晚到不同的分钟时，结局带上结果的时间。

### 3.5 不重复讲

有些经历此刻仍在模型眼前。Recall 指向它们，而不是重复：

- `key` 仍然生效的块（当前的规则或 state）讲作 `…（current version in your context）`；展开显示当时投递的那个版本；
- 最近一次的摘要就在上下文中；它的压缩那一行指出这一点，而不重复它。

同一个 key 以相同文本再次投递的块只讲一次。

### 3.6 轮次

一轮是同一个 `turn_id` 从 `turn/start` 到 `turn/end` 的跨度；它消费的输入属于它。任何轮次之外的事（两轮之间的压缩、fork）是轮次之间独立的行。索引中一轮的那一行给出它的第一段用户文本、调用过的工具及次数，以及 `turn/end` 中的结局（`completed`、`failed`、`cancelled`）。

## 4. 把手：时间与事件 id

每一行都带着其事件的时间（`create_at`）和事件 id。模型用 id 展开它。

- 事件 id 是 UUID。Recall 显示前 8 个十六进制字符，并接受在固定历史中唯一的任意前缀，就像 git 对待提交哈希那样；前缀有歧义时报错，并列出匹配的更长前缀。
- id 在本分支及共享前缀的子分支上永远有效。Timeline 序号做不到：fork 后历史的每一段都从 1 开始编号。
- 一轮的 id 是其 `turn/start` 事件的 id。

时间以 UTC 显示：每页开头和每次换日时有一行 `YYYY-MM-DD (UTC)`，各行只带 `HH:MM`。

## 5. 工具合同

### 5.1 声明

`toolContribution`，Session 作用域，键 `recall`（[session-boundary](../contracts/session-boundary.md)）：

| 字段 | 值 |
| --- | --- |
| `name` | `recall` |
| `risk` | `session` |
| `source` | `extension` |
| `description` | "Recall what happened earlier in this conversation: what the user said, what you did and what came back. With no arguments it lists past turns; open a turn or anything in it by its id, or search with about." |

工具声明是唯一的提示词。摘要、replacement 和系统提示词都不提这个工具（§7）。

### 5.2 输入

```json
{
  "type": "object",
  "properties": {
    "about":  {"type": "string", "minLength": 1, "maxLength": 200,
               "description": "Words to look for; all must appear, in any order, ignoring case."},
    "tool":   {"type": "string", "minLength": 1,
               "description": "Only calls to this tool."},
    "before": {"type": "string", "description": "An id; show what came before it."},
    "open":   {"type": "string", "description": "An id; show that turn, or that thing in full."},
    "offset": {"type": "integer", "minimum": 0,
               "description": "With open: continue from this character."}
  },
  "additionalProperties": false
}
```

由参数路由调用：

| 参数 | 结果 |
| --- | --- |
| 无，或 `before` | 轮次索引，从最近几轮那一页开始 |
| `about` 和/或 `tool`，可选 `before` | 所有轮次中命中的行 |
| `open` = 一轮的 id | 逐行讲述那一轮 |
| `open` = 一行的 id，可选 `offset` | 那件事的全文 |

`open` 与 `about`、`tool` 或 `before` 同时出现，或没有 `open` 却给了 `offset`，都是参数错误。

### 5.3 限制（暂定）

| 限制 | 值 |
| --- | --- |
| 整个 `content` | 最多 10,000 字符 |
| 轮次索引、检索 | 最多 20 行；每行最多 200 字符 |
| 一件事的全文 | 从 `offset` 起最多 8,000 字符 |

字符按 Unicode 标量值计，所以 CJK 文本不会在字符中间被截断。结果绝不超过上限：一页在会超限的那一行之前停下；超过一页的一轮用 `before` 继续。这些值与 #128 一起调整。

## 6. 模型看到什么

文本是普通的行，不带外层包装标签。`structuredContent` 只用于 UI 和持久化：`{level: "turns" | "turn" | "search" | "full", ids: [...], next?: {...}}`。

**轮次索引：**

```
2026-10-17 (UTC)
14:21 e41c07d2 turn: user "Stop. Don't upload anything yet, only write the files locally." — you called shell ×2 — completed
14:03 c2d8a915 turn: user "Use the staging bucket, never prod. Name files YYYY-MM-DD-report.csv…" — you called web_fetch, shell ×4, edit ×2 — cancelled
13:40 a7730e5f the conversation up to here was summarized (that summary is in your context)
...
Earlier: before=a7730e5f. Open one: open=<id>.
```

**一轮（`open=c2d8a915`）：**

```
2026-10-17 (UTC)
14:03 9b1e4d70 user: Use the staging bucket, never prod. Name files YYYY-MM-DD-report.csv, e.g. 2026-10-01-report.csv.
14:03 51c0aa3e you: I'll check the API limits first.
14:04 3f9a2c1b you called web_fetch url=https://example.com/api/limits → ok: "Rate limits: 600 requests per minute per key…"
14:10 8d2f6b04 rule AGENTS.md replaced (current version in your context)
14:12 c0e95a71 you called shell command="aws s3 cp report.csv s3://prod-reports/…" → denied by the user
14:20 77ab3e19 the user interrupted the turn
End of turn. Open one: open=<id>.
```

**一件事（`open=3f9a2c1b`）：** 一行标题 `14:04 3f9a2c1b you called web_fetch`，完整输入，然后完整结果；最后是 `Continue: open=3f9a2c1b offset=<next>.` 或 `End.`

| 结果 | 最后一行 |
| --- | --- |
| 轮次索引 | `Earlier: before=<id>. Open one: open=<id>.` 或 `Nothing earlier.` |
| 检索 | `More: before=<id>.` 或 `No more.`；完全没有命中：`Nothing matches <复述的过滤条件>.` |
| 一轮 | `End of turn. Open one: open=<id>.` 或 `More of this turn: open=<id> before=…` |
| 一件事 | `Continue: open=<id> offset=<next>.` 或 `End.` |

错误（`ToolResult::error`）说明怎么改正调用：`recall: open shows one turn or thing; drop about, tool and before.`、`recall: offset needs open.`、`No turn or line has id <prefix>.`、`<prefix> matches several: <更长的前缀>.`、`<id> has <len> characters.`；分支不可读时返回门面的错误。

旧内容以模型阅读的文本出现，而不是给它的指令：每行都标明是谁说了或做了什么，模型像对待任何工具结果一样对待它们。

## 7. 发现

模型通过工具声明知道这个工具，声明出现在配置了该工具的每个请求中。否决的方案：

- **在摘要提示词或 `summary_message` 中加一行，写明工具。** replacement 保存在持久的 `context/compaction` 记录中，工具名会被冻结进历史；摘要策略作为核心部件会因此依赖一个可选工具（设计规则 §3、ADR 002 D3）。
- **只在压缩之后才声明工具。** 工具表是 Session 级的，压缩是分支级的；这需要为 ToolRegistry 增加新机制（设计规则 §4 第 8 条）。而且 Recall 在压缩之前也有用：在很长的上下文里找出某条早先消息的原文。

## 8. 交互

| 情况 | 行为 |
| --- | --- |
| 后续压缩 | 什么都不移动：Recall 覆盖整个历史，id 永不改变。每次压缩只是多一行 |
| 调用期间发生压缩 | 在调用方分支上不会发生：压缩只在请求的准备阶段或作为 Loop 的一步运行，从不在工具批次运行时发生（[ContextCompaction §5.4](ContextCompaction.md#54-手动压缩)）。无论如何，固定的 head 保证结果一致 |
| Session 内的分支 fork | 子分支以相同的 id 回忆继承前缀，然后是它的 fork 那一行和它自己的事件 |
| Session fork 与删除 | fork 出的 Session 有自己的历史。关闭 Session 通过工具的取消令牌取消进行中的调用 |
| 中断 | `stop/interrupt` 取消调用的令牌。工具在读取之后、渲染之前检查它，并返回 `HostError::Cancelled` |
| 重载 | 工具不持有状态；进行中的调用结束，ToolExecutor 写其终态结果 |
| 回忆结果本身 | 一次回忆本身就是一次工具调用，以后的回忆像讲其他调用一样讲它 |
| 上下文增长 | 每次调用有界（§5.3）；其余由阈值压缩限制 |

## 9. 不做什么

- **不写入，不改类型。** 没有事件、`StateCell`、快照或广播。叙事投影是 Recall 自己的；`project_history` 及其类型不变。
- **不新增上下文输入路径。** 结果只以工具结果进入模型（AGENTS 第 10 条、设计规则 §4 第 6 条）。
- **不给原始事件，不讲机制。** 只讲 §3 列出的内容。
- **不读其他分支或 Session。** 分支取自 `ToolExecutionContext.branch_id`。
- **不做缓存。** 每次调用都读取固定的历史（§13）。

## 10. 安全与权限

- **风险类别 `session`。** 在 read-only 与 workspace-write 预设下直接允许，无需询问（[permission.rs](../../../../crates/dim-agent-capabilities/src/permission.rs)），与 `wait` 相同。它返回已经进入过本分支模型上下文的内容，以及 §3.2 中少数经历事件；不涉及文件、进程或网络。
- **信任边界：分支当前配置的模型。** 结果发往调用时所配置的 provider 和模型，它不一定是最初接收这些内容的那一个；分支可以采纳另一个模型（`session.agent.update`；[configuration.rs](../../../../crates/dim-app-server/src/agents/configuration.rs)）。这与分支已有的边界相同：切换之后，下一个请求会把整个投影上下文发给新的 provider。Recall 把这一边界扩展到已不在上下文中的事件，每次一个有界的调用。必须让早期内容不被另一个 provider 看到的用户，应为它新开一个 Session。
- **不可信内容。** 旧事件可能包含抓取的网页或注入的文本。它们以工具结果中带标签的行返回，与首次读取时的地位相同。
- **Tracing。** 除工具调用和结果事实外，没有其他内容。

## 11. 规则检查

| 规则 | 回答 |
| --- | --- |
| 设计规则 §4 第 1 条（改别人的状态写指令） | Recall 不改任何其他模块的状态。其结果由 ToolExecutor 写入。 |
| 设计规则 §4 第 2 条（交接） | 不适用：它不接手任何对象。 |
| 设计规则 §4 第 3 条（before 回调） | 没有。参数检查在工具处理函数内。 |
| 设计规则 §4 第 4 条（核心不带业务含义） | 不改核心。它按标签和 key 通用地读取运行时块，不按生产者区分。 |
| 设计规则 §4 第 5 条（原子批次） | 不适用：它不写入；读取固定在一个已发布的 head 上。 |
| 设计规则 §4 第 6 条（上下文只来自交接后的事件） | 它按上下文拼装的规则选取，只额外加入公开的经历事件；结果只以工具结果进入模型。 |
| 设计规则 §4 第 7 条（公开事件与服务） | 使用 `TreeCommands`、Branch 门面，以及公开事件和它们的关联 id。不读任何插件的内部状态。 |
| 设计规则 §4 第 8 条（基础设施不变） | Timeline、Dispatcher、信封、装配、门面和投影类型都不变。 |
| 设计规则 §4 第 9 条（只有后续决定要用的事实进 Timeline） | 不新增事件。 |
| 设计规则 §1（Capability 与 App Server 边界） | 是 `dim-agent-capabilities` 中的 Session Capability，由 App Server 安装在产品 profile 中。 |
| ADR 002 D1（作用域随状态） | Session：它提供工具，不持有状态；数据属于分支，每次调用读取。 |
| ADR 002 D2（不设接线模块） | 一个 `toolContribution`。 |
| ADR 002 D3（依赖方向） | 只依赖核心。它缺席时只少了这个工具（C3 省略规则）。 |
| ADR 002 D4（配置分离） | 没有业务配置；限制是暂定常量（§5.3、#128）。 |
| ADR 002 D5（合同语言中立） | 合同（§5、§6）是中立的声明和文本；Rust 形式见附录 A。 |

**影响面（公开内容）：** 一个工具名 `recall`。不新增事件、方法、wire 字段、服务，也不改类型。

## 12. 向机制所有者提出的需求

无。考虑过但不需要：门面上的分页历史读取（选取本来就需要整个前缀），以及按分支的工具声明（§7）。

## 13. 决定

| 项 | 决定 |
| --- | --- |
| 是什么 | 把分支历史讲给模型听，就像客户端里的会话视图 |
| 范围 | 整个分支历史，含继承前缀；压缩是一个事件，不是边界 |
| 投影 | 自己的投影：由一个起点事件和通过已有 id 挂上来的事件折叠成的事（§3.2）；按上下文拼装的规则判断是否算数，加上中断、权限变更、压缩和 fork |
| 工具调用 | 每次调用一行，带通用的参数简述和折叠后的结局：审批、未执行、后台任务、子 agent、错误或成功（§3.4） |
| 层级 | 轮次索引 → 一轮 → 一件事的全文；检索返回第 2 层的行 |
| 把手 | 每行带时间和事件 id；id 以唯一前缀显示 |
| 不重复 | 生效中的规则和 state、最近的摘要、流式片段、重复投递 |
| 提示词 | 只有工具声明（§5.1） |
| 风险类别 | `session` |
| 装配 | 只在产品 profile 中；`--models` 与 mock profile 不装 |
| 状态、写入、类型改动 | 无 |
| 成本 | 不做缓存：每次调用都读取并遍历固定的历史。只有测量表明需要时，才在工具内按分支和 head 加缓存；门面不变 |
| 交付 | 里程碑 M2（#129）；实现按 M2 的计划进行 |

## 14. 验收（流程测试）

流程测试放在 `crates/dim-agent-capabilities/tests`，使用测试策略；另加一个 `crates/dim-app-server/tests` 中的产品 profile 测试：

1. **轮次索引。** 无参数调用按最近在前列出各轮，带时间、id、第一段用户文本、工具及次数和结局；沿 `before=` 往前翻直到 `Nothing earlier.`。
2. **一轮。** 展开一轮时按顺序讲出用户原文、回复、每次调用连同其结局的一行，以及一次中断，各带时间和 id。
3. **一件事。** 展开一次调用的 id 显示其完整输入和结果，用 `offset` 分页直到 `End.`；CJK 文本从不被拆开。
4. **选取。** 已撤回和已删除的输入、thinking 和机制事件从不出现；运行时块按其标签和来源出现，key 仍生效的块说明当前版本在上下文中。`project_history` 放进上下文的每个事件都属于某件事。
5. **压缩。** 两次压缩之后，Recall 仍能回到第一轮；每次压缩是一行；压缩前返回的 id 在压缩后仍展开同样的内容。
6. **检索。** 顺序和大小写都不同的词作为 `about`，能找到第一次压缩之前的一条用户消息；`tool` 只保留该工具的调用。
7. **工具调用的结局。** 被拒绝、从未派发、转入后台任务且之后结束、由子 agent 回答、失败和成功的调用各一次，都按 §3.4 的结局讲成一行；steer 输入讲作用户在模型工作时插话。
8. **id。** 唯一的短前缀可以展开；有歧义的前缀列出更长的前缀；子分支可以展开继承的 id。
9. **边界与无副作用。** 没有结果超过 10,000 字符；Timeline 只新增 `step/tool-call` 与 `step/tool-result`。
10. **权限。** 在 read-only 预设下，工具不经审批请求即运行。

## 附录 A. Rust 绑定

```rust
// dim-agent-capabilities/src/recall.rs（新增）
pub const RECALL_TOOL: &str = "recall";
pub struct RecallPlugin { scope: SessionScope }
//   Deps = (InSession<TreeCommands>,)
//   apply: ctx.provide_as(scope.named_key::<ToolContribution>(RECALL_TOOL),
//                         ToolContribution { definition, handler: RecallTool(tree) })
struct RecallTool(Arc<TreeCommands>);
impl ToolHandler for RecallTool { /* §3–§6 */ }

// dim-app-server/src/main.rs，产品 profile（与 ProductSummaryStrategyPlugin 并列）
builder.session_plugin(RecallPlugin::new)
       .session_tool(RECALL_TOOL);
```
