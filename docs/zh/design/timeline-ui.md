# Timeline 优先的 TUI：branch 与 fork 从时间线派生

状态：提案。依赖 [Timeline branches](timeline-branches.md)（[#427](https://github.com/arcships/dim/issues/427)）与 [Session fork](appserver/session-deletion-and-fork.md)。范围：CLI TUI 的信息层级、交互，以及它在客户端折叠的数据；不改 runtime、事件、存储或 App Server 方法。English: [英文](../../design/timeline-ui.md)。

## 1. 问题

#427 之后 TUI 有三个互不相关的入口：对话本身（`main` 的 transcript）、与 `/tasks` 平级的 `/lines` 面板，以及 TUI 里完全没有入口的 `session.fork`。用户看不到一条线从哪里长出来，也不能从历史中的某一点开线（#427 S3）。

在数据上这是同一件事：**branch 和 fork 都是在一条 Timeline 的某个序号处切割。** `branch/create` 带 `at`，`session.branch_create {parentBranchId, at, prompt}` 与 `session.fork {sessionId, at}` 都以 `at` 为参数。所以 Timeline 是主轴，branch 和 fork 是在它某一点上的动作，lines 面板是这些动作结果的地图。

## 2. 层级

```
L0  Session ............ 主体：身份、工作区、交互面
 └ L1  Timeline ........ 当前显示的那条线的 transcript；唯一的主界面
     ├ 点 .............. 切点：某条用户消息之前的回合边界
     │   ├ e 编辑 / q 引用         （已有）
     │   ├ b 从这里分支 → 本 Session 的一条线
     │   └ f 从这里 fork → 一个新 Session（仅 main）
     └ 标记 ............ ⑂ 在这里打开了一条线（可选中）
 L3  Lines（/lines）.... 每条线挂在它被切出的父线下
```

1. **只有一个主界面。** 屏幕永远显示一条线的 Timeline。换线 = 换一条 Timeline 看，不换界面。
2. **派生从点出发。** branch 和 fork 都从当前 Timeline 的某个点开始；它的末尾也是一个点（`/branch`、`/fork`）。
3. **结果回到 Timeline。** branch 在父线上留下 `⑂` 标记，屏幕跟到新线；fork 进入新 Session 的 Timeline。

## 3. Branch 与 fork

两者共享"点 → 派生"的入口，区别在结果住在哪里（#427 §2）：

| | Branch（`b`） | Fork（`f`） |
| --- | --- | --- |
| 结果 | 本 Session 的一条线 | 一个新 Session |
| 共享 | 工作区与同一个主体；与父线互发消息 | 只共享切点之前的历史 |
| 可用的点 | 任一未结束的线，合法的上下文边界 | 仅 `main`，且必须是 rest point |
| 之后 | 屏幕显示新线 | TUI attach 到 fork |
| 在源上的痕迹 | `⑂` 标记 | 无（§7） |

不适用的动作用 notice 说明原因（`A fork copies main; b opens a line here`、`Line has ended: X`）。切点是否合法只由 App Server 判断，拒绝时显示为 notice。

## 4. Timeline 与点

### 4.1 点

fullscreen 的 Tab 选取本来就停在用户消息上。被选消息的点是它之前的那个序号，即"这条消息还没说出口的时候"。它是原子批次的结尾，通常满足 `validate_context_boundary`，在 `main` 上通常也是 rest point；在这里换一种说法分支，正是 S3 的重问与对比。

当前线的末尾也是一个点：它最近结束的回合，供 `/branch`、`/fork` 使用。还没有结束的回合时没有点（`No finished turn to cut at yet`）。

```
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ › 把 cache 的锁换成 RwLock 试试                            ┃
┗━e 编辑 · b 分支 · f 派生━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
```

框太窄放不下时只显示 `e 编辑`。

### 4.2 `b`：从这里分支

`session.branch_create` 需要新线的开场输入，所以 `b` 把被选消息放进 composer，composer 的下一次提交就打开这条线：

```
╭──────────────────────────────────────────── provider / model ─╮
│ › 把 cache 的锁换成 RwLock 试试█                               │
╰──────────────────────────── ⑂ 从 main 开新分支线 · Esc 取消 ─╯
```

`Enter` 发送 `session.branch_create {parentBranchId: 当前线, at, prompt}`，屏幕显示新线；`Esc` 取消；切换会话或线会丢弃草稿。

### 4.3 `f`：从这里 fork

仅 `main`。composer 底边询问 `在这条消息之前派生会话？(y/N)`；`y` 发送 `session.fork {sessionId, at}`，在 `已派生 ID · N 条消息` 边界下 attach 到 fork，并把被选消息放进它的 composer。fork 在用户提交之前不开始任何工作（fork 设计 §6.5）。

### 4.4 标记

`branch/create` 在线打开的那一刻写在父线上，对过去的切点来说就是父线的末尾，而不是切点处。已提交的 scrollback 只能追加，所以标记留在事实所在的位置，并写明它的切点：

```
 › find the leak
 … 回答 …
 › try the parser
 … 回答 …
 ⑂ line f7f0a984 · before “try the parser” · try the parser via cache
```

切点恰好在本 transcript 的某条消息之前时才写 `before “…”`；在末尾打开的线（`branch` 工具调用）没有。标记和成员卡片一样是 Tab 的停靠点：`Enter` 或点击显示那条线。结束不做标记：lines 面板会显示，`main` 也会收到它的工具线的通知。

### 4.5 我在哪里

显示一条线时，composer 底边写出从 `main` 到它的路径和切点：

```
╰── ~/repo (main) · line main ▸ b-91 ▸ b-c3 · after message 4 ─╯
```

`after message N` 是切点时父线历史中的用户消息数，包括它继承的部分。

## 5. 切线

与 #427 §14 相同：切换时写一行 `line ID` 边界，然后是该线的 transcript，只含它自己的事件（每个分支各自编号，[sessions service D11](appserver/sessions-service.md)）。它继承的历史属于父线，离它只有一个标记的距离。

## 6. Lines 面板

`/lines` 是 Timeline 的地图。子线先按它在父线上被切出的位置排序，再按打开顺序，每行写 `after message N`：

```
┌ Lines ───────────────────────────────────────────────────────────┐
│   main        the session itself                                 │
│     b-02      after message 1 · docs pass · ended itself         │
│     b-7c      after message 3 · parser race · working · shown    │
│       b-c3    after message 4 · the guard in cache.rs · working  │
└──────────────────── Enter show the line · x end it · Esc close ──┘
```

fork 是别的 Session，归 `/resume`。按键保持 #427 §14 的定义。

## 7. 客户端数据

| # | 需求 | 归属 | 说明 |
| --- | --- | --- | --- |
| R1 | `LineView.cut`：父线序号及该处的消息数 | CLI projection | 由 `branch/create.at` 与父线的用户输入折叠而来；无格式变化 |
| R2 | 用户消息行的切点（`ViewRow.cut`）与标记行的线（`ViewRow.line`） | CLI projection | 来自客户端已有的序号 |
| R3 | `session.branch_create` 与 `session.fork` 调用 | CLI session host | 已有方法；从过期 attachment 发出的请求在本地拒绝 |
| R4 | 后续、可选：显示 fork 的来源（源上的 `↗ forked`，fork 里的 `forked from S`） | App Server，需格式决定 | `session/forked` 不带数据，fork 设计也不加来源列 |

## 8. 不做的事

- **改写 `main` 的历史。** #427 §3：不是 branch。"从这里重来"是 `b`（在本 Session 里对比）或 `f`（新 Session）。
- **Compare / adopt。** S3 的后半段需要单独设计；同一切点下的兄弟线就是对比要展示的东西。
- **画出 DAG。** scrollback 只能追加；拓扑由标记、面板和路径表达。

## 9. 消融检查

- 没有新面板或模式：点沿用 Tab 选取，线标记沿用卡片选取，切线沿用 #427，fork 后的 attach 沿用 `/resume`。
- 新增的交互：选取上的 `b`/`f`、composer 的分支草稿与 fork 询问、路径式底边、`/branch` 与 `/fork`，各自对应 §2 的一条规则。
- 考虑过但去掉的：lines 面板里的 `b`（需要每条线的末尾；当前线上的 `/branch` 已覆盖）、按点置灰的提示（按键时的 notice 说的是同一件事，状态更少）、结束标记（§4.4）、继承前缀的一行摘要与 `Esc Esc` 回父线（§5 和标记已经通向那里）。

## 10. 决定

1. 点是用户消息之前的边界，加上线的末尾；不提供回答之后的边界。
2. `b` 和 `f` 都像 `e` 一样把被选消息带进 composer。
3. R4 等待格式决定。
