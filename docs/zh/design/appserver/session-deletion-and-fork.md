# SessionStore Session 删除、清理与分叉

状态：草稿（待评审）。路线图工作项 S5c（[#61](https://github.com/arcships/dim/issues/61)），在 S5a
（[#59](https://github.com/arcships/dim/issues/59)）及其设计 [Session 文件](session-files.md) 之后。
总体设计：[附录 B.2](../overview.md#b2-app-server-进程插件) 的 Rpc、Agents 与 SessionStore 行。作用域：
[ADR 002](../../adr/002-plugin-scope.md) 进程作用域，位于现有的 Agents 与 SessionStore 插件内，另加两个 Rpc 内建方法。
使用方：通过 `session.delete` 和 `session.fork` 调用的客户端，以及 CLI 的 `dimcode session` 命令（F6，
[#77](https://github.com/arcships/dim/issues/77)）。

## 1. 概要

客户端用 `session.delete` 删除 Session，用 `session.fork` 分叉 Session。两者都是协议 v3 的 Rpc 内建方法，调用 Agents；Agents 负责加载、关闭和创建 Session，SessionStore 执行行与文件的步骤。

- **删除。** Agents 不再加载该 Session；若它已加载，就关闭它，让各能力结算在途工作并释放所持资源。随后 SessionStore 移除该 Session 的命令输出目录，并在一个事务内删除存储为它保存的所有行，包括 Session 文件的所有者行。失去最后一个所有者的文件由 S5a 的清扫移除。删除不存在的 Session 也成功。
- **清理。** 一次删除移除该 Session 拥有的一切（§3.3）。按时间或保留数量清理并预览，由 CLI 组合 `session.summaries` 与 `session.delete` 完成，没有对应的服务端方法。
- **崩溃安全。** 目录的移除在行事务提交之前落盘，Session 文件按 S5a 的行处理。删除在任何一步中断，都只会留下完整的 Session 或者它的行全部消失，重试或下一次清扫会完成剩余工作（§4）。
- **分叉。** 分叉在一个事务内把源 Session 的 `main` 截至某个切点的部分，连同 `main` 到那时创建的分支，按存储的字节复制到一个新 Session，并共享源的 Session 文件。分叉的 `main` 以 `branch/resume` 打开：分叉就是源的对话在切点处、以新 id 恢复。`SessionConfig` 整体复制，`interactive` 和权限预设随之带过去，不放宽任何权限。
- **格式。** 在 S5a 的 v15 之外不改格式：不加来源列，不写墓碑，不加事件。

不新增 Timeline 事件，不改事件类型或格式样本，也不改运行时、TimelineTree、Prompt、rutis 或 Session 门面。

## 2. 结构

```
 客户端 -- session.delete {sessionId} ----------+
        -- session.fork {sessionId, at} --------+
                                                v
 进程作用域（控制节点）                Rpc 内建方法（v3）：参数格式、协议版本
                                                |
                                                v
                         Agents（AgentIndex）：每个 Session 的准入门、
                         关闭、从另一个 Session 的切点创建
                           |                                 |
                           | delete_session(S)               | 预备创建，
                           |                                 | 预备步骤 = fork_session(S, F, at)
                           v                                 v
                         SessionStore ---- 各一个事务 ----> SqliteStorage
                           |      \
                           |       +-- S5a 清扫（Session 文件）
                           | 每次调用按 Session 的资源节点查找（#38）
                           v
 资源节点（P1/P2 即控制节点）              FileStorage: remove(command-output/<S>)
```

Rpc 检查参数和 Session 的协议版本，然后调用 Agents。Agents 负责 Session 的生命周期：`acquire` 恢复 Session 时已经持有的准入门、`Session::close` 和预备创建（`create_prepared_session`）。SessionStore 负责数据库目录下的行和文件。它新增的步骤是作用于原生存储的 Rust 方法，不是服务：只有 Agents 调用。

## 3. 删除

### 3.1 方法

`session.delete {sessionId: string} -> {}`

| 项目 | 规则 |
| --- | --- |
| 类别 | Rpc 内建方法，与 `session.create`、`session.close` 并列。它不是 `sessionMethods` 条目，因为 Session 不能通过自己的部件卸载自己；也不是管理方法：它作用于一个 Session，Rpc 对 Session 的检查都适用 |
| 协议 | v3。v1 与 v2 得到 `unknown_method`（"Method requires protocol v3"）。`hello.features` 在列出 `session.create` 的条件下列出它。协议版本仍为 3：客户端通过 `hello.features` 发现新的内建方法，与发现 `session.summaries` 相同 |
| 调用方 | 能调用 `session.close` 的连接都能调用，配置规则相同：受保护配置下只限本地传输。已配对设备（P3）在 RemoteGateway 的兼容性表中与其他 Session 内建方法一起决定 |
| Session 版本 | Session 记录的协议版本必须等于连接的版本（`session_protocol_version`），与所有 Session 内建方法相同。没有可读所有者行的 Session 没有记录版本，检查通过 |
| 已损坏的 Session | 按 id 删除。删除不读取 Session 的任何记录，因此校验失败或没有可用所有者的 Session 也能删除；Rpc 对不可用 Session 的检查不适用于 `session.delete` |
| 回复 | Session 的行全部删除后回复 `{}`。未知或已删除的 id 也回复 `{}` |

### 3.2 顺序

```
 Rpc          Agents                        Session S            SessionStore              FileStorage
  | delete(S) ->|                              |                     |                          |
  |             | 1 取得 S 的准入门            |                     |                          |
  |             | 2 已加载？close() ---------->| 结算已接受的工作，写终态事实，
  |             |                              | 释放进程、租约、成员请求；
  |             |<-------------- 已关闭 -------| trace 归档结束      |                          |
  |             | delete_session(S) ------------------------------->|                          |
  |             |                              |  3 移除 command-output/S，同步父目录 --------->|
  |             |                              |    不存在视为已移除 <--------------------------|
  |             |                              |  4 在 Timeline 写线程上的事务：所有者行、      |
  |             |                              |    Timeline 行、owner、记录 -> 提交            |
  |             |                              |  5 安排 S5a 清扫                               |
  |             | 6 忘记 S；保持其槽位关闭     |                     |                          |
  |<---- {} ----|                              |                     |                          |
```

1. **准入。** Agents 取得 S 的准入门，即 `acquire` 恢复 Session 时持有的 `SessionSlot` 门，删除进行期间没有请求、订阅或分叉能加载 S。已经持有 S 使用权的请求继续持有，由第 2 步的关闭结束其工作。
2. **关闭。** 若 S 已加载，Agents 像 `session.close` 那样关闭它：业务关闭，不是卸载。各能力结算已接受的工作，写出终态事实，释放进程、FileStorage 租约和成员请求；删除也像 `session.close` 那样等待 trace 归档结束。关闭报告清理错误时记日志并继续删除：未能写出的事实会随其余部分一起删除。
3. **命令输出。** SessionStore 通过 S 的资源节点上的 FileStorage（与 S5a 一样每次调用时查找）移除 `<数据库目录>/command-output/<S>/`，WorkspaceTools 在这里保存 exec 输出和过大的 `read` 结果（[read.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/files/read.rs) 的 `offload`）。目录不存在视为已移除。移除必须在第 4 步之前落盘（§4），因此 FileStorage 的 `remove` 要同步被移除条目的父目录（§14）。失败时删除在改动任何行之前结束（§3.5）。
4. **行。** 一个 `IMMEDIATE` 事务删除 S 的全部行：`session_file_owners`、`timeline_snapshots`、`timeline_events`、`timeline_branches`、`timeline_content`、`session_owners` 和 `agent_sessions`，因外键按此顺序，另加 S5b 的标题元数据（§14）。这就是如今创建回滚执行的操作（[timeline.rs](../../../../crates/dim-app-server/src/storage/timeline.rs) 的 `rollback_creation`），S5a 为它加上所有者行；删除与回滚共用它。它像回滚一样在 Timeline 写线程上执行，因此排在已为 S 排队的所有追加之后，并清除 S 记下的写入失败。
5. **清扫。** SessionStore 安排 S5a 的清扫（[Session 文件 §5.4](session-files.md#54-清扫)）。最后一个所有者是 S 的文件在清扫中移除；仍有分叉拥有的文件保留。
6. **忘记。** Agents 删除 S 的协议版本和不可用 Session 条目，并保持 S 的槽位关闭，`acquire` 因此回答 `session_closed`。在第 4 步之前失败的删除则释放准入门，S 在下次使用时照常加载。

### 3.3 删除移除什么

| 状态 | 位置 | 由谁移除 |
| --- | --- | --- |
| Session 记录、所有者、分支、事件、内容、快照 | 原生存储 | 第 4 步，一个事务 |
| Session 文件的所有权 | `session_file_owners`（S5a） | 第 4 步。失去所有者的文件由 S5a 的清扫处理 |
| 标题元数据 | 原生存储，S5b 的设计 | 第 4 步，同一事务 |
| exec 输出和转存的 `read` 结果 | `<数据库目录>/command-output/<S>/` | 第 3 步 |
| 已加载实例、租约、订阅、在途使用 | 进程内存 | 第 2 步 |
| 协议版本、准入槽位、不可用 Session 条目 | Agents 内存 | 第 6 步 |
| 指向 S 的 Collaboration 关系 | Collaboration 内存 | 它现有的关闭观察者，已经把关闭的 Session 当作删除（[collaboration.rs](../../../../crates/dim-app-server/src/collaboration.rs) 的 `session_closed`） |

删除不移除：

- **trace 归档。** 它写在 `session.create {trace}` 指定的位置，路径交给那个客户端，归客户端所有。第 2 步等待归档写完。
- **其他所有者记录中指向 S 的内容**，例如 Scheduler 与 Workflow 存在 `pluginFiles` 中的运行记录。这些所有者使用该 id 时会得到 `session_closed`，与 Session 已关闭时相同。
- **其他 Session。** 被删除的 leader 的成员、S 的分叉都是有自己行的 Session。删除不级联。
- **宿主配置**（按工作区的 Provider 选择、MCP 项目启用），不属于任何 Session。

### 3.4 客户端与订阅

- 第 2 步关闭 S 时，附着在 S 上的连接收到现有的 `session.state` 和 `session.closed` 通知，与 `session.close` 相同。不加原因为 "deleted" 的通知：`session.closed` 在行事务提交之前发出，可能宣告一次随后失败的删除。协议也没有新建 Session 的列表通知，所以删除也不加。
- 回复之后，对 S 的 `session.attach`、Session 方法和 `session.fork` 都失败于 `session_closed`，`session.list` 和 `session.summaries` 不再列出它。
- 发出删除的连接本身也可能附着在 S 上，它先收到通知，再收到回复。

### 3.5 幂等与错误

每一步都容许目标已不存在，因此任何失败或回复不确定之后都可以重试删除。

| 结果 | 何时 | 之后的状态 |
| --- | --- | --- |
| `{}` | 行已删除，或本来就没有 | Session 不存在；失去所有者的文件由清扫处理 |
| `unknown_method`（-32601） | 协议 v1 或 v2 | 不变 |
| `invalid_params`（-32602） | 缺少 `sessionId` 或不是字符串 | 不变 |
| `session_protocol_version` | S 属于另一个协议版本 | 不变 |
| `product_integration_pending` | 该配置不提供 Session 执行 | 不变 |
| `-32603` "Session operation failed" | 第 3 步失败（`Locked`、`DirectoryNotEmpty`、`Unsynced`、I/O、节点不可用）或第 4 步失败（SQLite） | S 已关闭但未删除：行还在，下次使用时照常加载，命令输出可能已部分移除。重试 |
| 没有回复 | 回复之前连接或 Rpc 已结束 | 两种状态之一。重试 |

第 3 步的 `Locked` 和 `DirectoryNotEmpty` 只会来自已关闭 Session 之外的写入方，例如比 shell 活得更久的 PowerShell 子进程，或打开着文件的其他进程。删除失败，等它们退出后重试。

## 4. 崩溃收敛

```
            2 关闭             3 移除（已同步）             4 提交                5 清扫
 [已加载] ---------> [已关闭] -------------------> [无目录] -----------> [无行] ----------> [消失]
    ^                    |                              |                     |
    |                    | 崩溃：行都在，               | 崩溃：行都在，      | 崩溃：无所有者的
    +--- 下次使用 -------+ 目录部分移除                 | 目录已无            | 文件行（S5a）
         加载 S            重试删除                       重试删除              下一次清扫
```

| 中断点 | 存储 | 磁盘 | 收敛方式 |
| --- | --- | --- | --- |
| 关闭之前或期间 | 行都在 | 目录在 | 没有损失；S 下次使用时加载；重试 |
| 移除期间 | 行都在 | 目录部分移除 | 重试：继续移除，不存在的条目视为已移除 |
| 移除之后、提交之前 | 行都在 | 目录已无，已落盘 | 重试。期间 S 可以加载，只是没有命令输出文件 |
| 提交之后、回复之前 | 无行 | 目录已无；S 的文件无所有者 | 客户端重试得到 `{}` 并再次安排清扫；否则由 S5a 的下一次触发（另一次删除，或之后某一代的第一次写入）移除文件 |
| 清扫期间 | S5a 的状态 | — | [Session 文件 §6](session-files.md) 的崩溃恢复表 |

**为什么目录在提交之前移除。** 没有任何行记录命令输出目录。行删除之后，孤立的目录只能靠列出 `command-output/` 找到，而这个目录由同一数据库目录下的所有数据库共用（一个目录下两个 `--database` 文件），列目录无法区分已删除 Session 的目录和另一个存储的目录。先移除目录，任何已提交的删除都不会留下它。S5a 的文件采用相反的顺序，因为有行记录它们，清扫能找到。FileStorage 的移除不同步（[S6](https://github.com/arcships/dim/pull/87)）：断电后，被移除的目录可能重新出现，而之后以 `synchronous=FULL` 提交的事务仍然有效。提交之前同步父目录就消除了这个窗口。

被否决的方案：

| 方案 | 原因 |
| --- | --- |
| 先提交，再移除目录 | 两者之间崩溃会留下没有任何东西指向的目录，而在共用的数据库目录里列目录并不安全 |
| 用墓碑行记录待清理的已删除 id | 为一个目录改格式（v16），而上述顺序不需要它就能收敛 |
| 用卸载代替关闭 | 卸载不结算已接受的工作，也不停止成员请求（AGENTS.md 第 8 条） |

## 5. 清理

### 5.1 按时间或数量清理

[功能清单](../../../feature-list.md)要求"按时间或保留数量清理，并可预览"。由 CLI 组合实现：`session.summaries` 为连接所属协议版本的每个 Session 返回 `createdAt`、`updatedAt` 和预览；CLI 选出要删的 Session、展示所选内容，再逐个调用 `session.delete`。失败的 id 会报告出来并可重试，其余照常删除。

被否决：`session.cleanup {olderThan, keep, dryRun}` 这样的服务端方法。它重复了客户端已有的选择和预览；保留策略需要一个业务所有者和配置，而目前没有任何定义；部分失败时仍要逐个 id 返回结果。

### 5.2 孤立物

| 类别 | 处理 |
| --- | --- |
| Session 文件 | S5a 的清扫：每次删除之后（包括重试的删除），以及每一代的第一次写入 |
| 命令输出 | 不需要：提交之前已移除（§4） |
| 已删除 Session 的行 | 不需要：一个事务 |
| 以前的故障留下的行，例如没有记录的 Timeline 行 | 按该 id 删除即可移除：每张表都按 `session_id` 清除，不论 `agent_sessions` 中是否有该行 |

## 6. 分叉

### 6.1 方法

`session.fork {sessionId: string, at: u64} -> {sessionId: string, sequence: u64}`

| 项目 | 规则 |
| --- | --- |
| 类别 | Rpc 内建方法。它像 `session.create` 一样创建 Session，只是从另一个 Session 的切点创建（Agents） |
| 协议与调用方 | 与 `session.delete` 相同（§3.1）。源必须属于连接的协议版本，分叉也采用该版本 |
| `at` | 分叉继承的最后一个 `main` 序号。0 表示不继承任何事件 |
| 回复 | 分叉发布后的 id 及其 `main` 头部。客户端随后附着 |
| 没有其他参数 | 没有 `branchId`、配置字段或 `trace`。分叉的配置就是源的配置（§6.3）。客户端之后用 `session.permission.update` 或 `session.model.update` 修改，与 [#26](https://github.com/arcships/dim/issues/26) 对预设的决定相同 |

分叉不是幂等的：回复不确定后重试可能再建一个分叉，客户端可在 `session.summaries` 中找到它。

### 6.2 切点

切点在 `main` 上，规则与 `session.branch_create` 相同（[tree.rs](../../../../crates/dim-app-server/src/connection/tree.rs) 的 `create_branch`）：`at` 不超过已发布的 `main` 头部，位于原子批次的末尾，并且 `history::validate_context_boundary` 接受 `at` 处的上下文（没有缺少结果的工具调用，没有未结束的模型输出）。Agents 获取源 Session 来做这项检查，需要时先恢复它，与 `session.branch_create` 相同。`at` 及之前的行不会改变，分叉期间的追加不影响复制。

**为什么只在 `main` 上。** 子分支的历史是各祖先的前缀加上它自己的事件，而每个分支的事件都从 1 编号。把它排成一条 `main` 要给事件重新编号，而压缩的 `through`、`branch/create.at`、`branch/start.createEvent`、批次终点和快照锚点都引用这些编号：存储将不得不改写其他所有者写下的事实。要继续某个子分支的工作，客户端在 Session 内从它创建分支。

被否决：以相同的分支 id 复制子分支的祖先链，并在子分支的副本上继续。那样分叉的对话就不在它的 `main` 上，而客户端、`session.list` 和 SessionTitle 读的都是 `main`。

### 6.3 分叉复制什么

```
 源 S                                                     分叉 F（一个事务）
 main: 1 .. 7  8 branch/create(c1)  9 .. 12 | 13 ..       main: 1 .. 12，id 相同         (at = 12)
                      |                  切点                   之后是 13 branch/resume，由 F 写入
 c1:   1 branch/start .. 30（可能仍在增长）              c1:   1 .. 30，按已提交的整体复制
 c2:   在 main 15 处 branch/create                       -     不复制（在切点之后创建）
 timeline_content: 事件引用的行                          只复制被复制事件引用的行
 timeline_snapshots: main <= 12、c1                      复制
 session_file_owners: (S,f1) (S,f2)                      (F,f1) (F,f2)，不复制字节
 agent_sessions.config: 存储的文本                       相同的文本
 session_owners: (S, Agent A, v3)                        (F, Agent A, v3)
 S5b 标题元数据                                          不复制
```

| 表 | F 得到的行 |
| --- | --- |
| `agent_sessions` | 新 id（UUID v4）、分叉时刻的 `created_at_ms`，以及原样的源 `config` 文本，也就是整个 `SessionConfig`，包括 `interactive`（[Session 访问 A.3](../contracts/session-access.md#a3-sessionconfig-字段)）、`permissionPolicy`、`collaborationRole`、`cwd`、`model`、`systemPrompt` 和 `tools`。复制文本而不重新序列化，`interactive` 的省略规则保持原样 |
| `session_owners` | 源的 Agent 与协议版本 |
| `timeline_branches` | 头部为 `at` 的 `main`；`branch/create` 位于 `main` 上 `at` 及之前的每个分支，以及其下的所有分支，保留存储的 `info` 与头部 |
| `timeline_events` | `main` 截至 `at` 的行（切点规则保证没有行跨过它），以及被复制子分支的全部行；种类、正文、序号、批次终点、事件 id 和 before 轨迹都按存储的样子 |
| `timeline_content` | 被复制事件按编解码器写入的引用（请求及其 before 轨迹）所引用的行 |
| `timeline_snapshots` | 锚定在被复制事件上的行。它们由相同的事实派生，F 首次加载时可以直接使用，不必从头重放 |
| `session_file_owners` | S 拥有的每个文件给 F 一行（[Session 文件 §4](session-files.md#4-分叉时共享)） |
| 标题元数据（S5b） | 不复制：分叉没有标题（[SessionTitle](../plugins/SessionTitle.md) A7） |

**为什么子分支整体复制。** 存储要求每个 `branch/create` 都有对应分支的目录，并在加载 Session 时检查（[timeline.rs](../../../../crates/dim-app-server/src/storage/timeline.rs) 的 `validate_session`），TimelineTree 也会恢复每个存储的分支。没有目录，F 无法加载；目录为空，TimelineTree 会在 F 的 `main` 上为一次成功的创建写下 `branch/create_failed`，理由是 "interrupted by process restart"。分支的事件与 `main` 的切点之间没有先后顺序，子分支没有哪个前缀对应 `at`；因此按分叉事务执行时已提交的样子整体复制，F 像重启后一样恢复它（§6.5）。

**为什么只复制被引用的内容。** 内容行按 Session 存放。全部复制的话，从长 Session 的早期分叉会把切点之后所有请求的内容也复制过去，而复制期间一直持有存储的写锁。

被否决的方案：

| 方案 | 原因 |
| --- | --- |
| 保留子分支目录、丢弃其事件 | TimelineTree 会为一个运行过的分支记下理由不实的创建失败 |
| 子分支只复制到创建为止 | 它的第一个 Queue 输入重新变成待处理，F 会把委托的任务再执行一遍 |
| 复制源的全部内容行 | 为切点之后、没有被复制事件引用的请求付出磁盘和锁的时间 |

复制是一个写事务；所有 Session 的提交都要等它，就像等任何一轮写入一样，时长随被复制的历史增长。

### 6.4 分叉不复制什么

| 不复制 | 原因 |
| --- | --- |
| `at` 之后的 `main` 事件，`at` 之后创建的分支 | 在切点之后 |
| 已加载实例、租约、订阅、已附着的客户端、在途请求 | 运行时状态，不存储；F 自己建立 |
| 正在提交中的输入 | 尚未提交，不是事实 |
| 命令输出目录 | WorkspaceTools 的每 Session 目录。F 复制来的工具结果引用 `command-output/<S>/` 下的路径，S 存在期间可读（§6.6） |
| 标题元数据 | SessionTitle A7 |
| Collaboration 关系和临时 Agent | 源团队的进程内存。F 不领导任何成员，S 的临时 Agent 对 F 不可见 |
| 宿主配置 | 从不属于 Session |

### 6.5 打开分叉

Agents 用现有的预备创建来创建 F：预备步骤是 SessionStore 的分叉事务，而不是 `create_owned_session`。随后运行时校验 F 的行并构建 F。失败时回滚 F，包括它的所有者行（[Session 文件 §5.3](session-files.md)）；源不受影响。

TimelineTree 以 `branch/resume` 打开已有事件的根分支（[timeline_tree.rs](../../../../crates/dim-agent-capabilities/src/timeline_tree.rs) 的 `restore`），因此 F 的 `main` 在复制来的事实之后接着写 `branch/resume`。F 就是源的对话在切点处、以新 id 恢复，每个所有者按现有的重载规则处理：

| 切点处 | 在 F 中 |
| --- | --- |
| 模型配置（`context/configured`）、提示词、已采用的工具 | 保留。恢复的分支保留其配置，Prompt 像任何恢复时一样投递来源变化，因此提示词前缀与 S 在切点处相同 |
| 权限预设（`permission/changed`） | 保留。Permission 折叠复制来的事实（#26）；`SessionConfig` 中的策略也一并复制 |
| 未结束的轮次 | Loop 以重载中断结束它；不会重新执行任何东西 |
| 被持有的输入 | 在 F 中同样被持有，像重启后一样被采纳 |
| 没有 `exec/ended` 的后台任务 | WorkspaceTools 写出其 `failed` 终态 |
| 已送达但没有结果的 Collaboration 请求 | Collaboration 写出失败结果，但不得停止源的成员（§14） |
| 生效中的目标（Goal，P5） | Goal 的 A11（§14） |

被否决：在复制来的事实之后写 `branch/start`。Prompt 把根分支的 `branch/start` 当作新 Session，会采用 `SessionConfig` 中的模型和系统提示词，从而撤销复制来的事实中的模型变更，并改变提示词前缀（[prompt.rs](../../../../crates/dim-agent-capabilities/src/prompt.rs) 的 `open`）。要避免这一点，得改 TimelineTree（让已有事件的预备根分支以 start 打开）和 Prompt（区分分叉的 start 与新 Session 的 start）。而且 F 提交之后、写出 start 之前崩溃，F 仍会以 `branch/resume` 打开。

### 6.6 身份、来源与路径

- **事件 id。** 复制的事件保留原 id。事件由所属 Session 和 id 共同标识：存储按 Session 检查 id，并按 `(session_id, event_id)` 建索引。换新 id 就要改写所有引用它们的地方：`input_id`、`call_event_id`、压缩游标、before 轨迹、快照锚点和 Collaboration 的操作 id。
- **其他 id。** 分支、轮次、请求、调用、任务和审批 id 都保留，它们同样按 Session 划分。复制来的任务 id 在 F 中没有对应的运行任务，`session.exec.stop` 回答任务未知。
- **`submission_id`。** 它随 `input/message` 复制，`session.input_lookup` 读取 F 的历史，因此能在 F 上找到复制来的输入。[#32](https://github.com/arcships/dim/issues/32) 移走这个字段时要保持这一点（§14）。
- **来源。** F 不记录源：没有列，也没有事实。目前没有任何决定使用它；Session 访问 A.3 规定 Session 的来源（`createdBy`）在有使用方时连同其格式决定一起加入。
- **路径。** Session 文件通过所有者行共享，只要还有所有者就有效（S5a）。命令输出不共享：F 复制来的工具结果引用 `command-output/<S>/…`，S 存在期间 F 可以读取。S 删除之后，读取这些路径会得到文件不存在，工具结果中的预览仍在。

被否决：在 S 的分叉都删除之前保留 S 的命令输出。删除需要知道 S 有哪些分叉，就得在存储中加分叉关系（改格式），WorkspaceTools 的每 Session 输出也会多出它本没有的所有者。

### 6.7 错误

| 结果 | 何时 | 之后的状态 |
| --- | --- | --- |
| `{sessionId, sequence}` | F 已发布 | F 存在 |
| `unknown_method` | 协议 v1 或 v2 | 什么都没创建 |
| `invalid_params`、`session_invalid_input` | 参数格式，或 `session.branch_create` 在同一位置会拒绝的切点 | 什么都没创建 |
| `session_closed` | 源未知或已删除，包括源的删除在分叉事务之前提交 | 什么都没创建 |
| `session_protocol_version` | 源属于另一个协议版本 | 什么都没创建 |
| `session_build_failed` | F 构建失败 | F 已回滚 |
| `-32603` "Session operation failed" | 存储失败，包括 F 复制来的行校验失败 | 未提交任何东西，或 F 已回滚 |

## 7. 串行化与竞争

删除和分叉在唯一的原生存储连接的写事务上决定，与 [Session 文件 §6](session-files.md) 中的写入相同。Agents 的准入门让删除与加载有先后。

| 竞争 | 结果 |
| --- | --- |
| 分叉先提交，然后源被删除 | F 保留复制的数据和共享的文件 |
| 源的删除先提交，然后是分叉事务 | 分叉找不到源：`session_closed`，什么都没创建 |
| 删除的关闭发生在分叉检查切点期间 | 检查失败于 `session_closed` |
| 写入与删除或分叉 | Session 文件 §6 |
| 两次删除 | 第二次等准入门，然后发现没有行：`{}` |
| 删除与 attach、submit 或 Session 方法 | 它们等准入门，然后失败于 `session_closed`；已持有使用权的请求随关闭结束 |
| 删除与该 Session 自己的追加 | 关闭结算它们；行事务在 Timeline 写线程上排在所有已排队的追加之后 |
| 分叉与对源的追加 | `at` 及之前的行不变；子分支按分叉事务执行时已提交的样子复制 |
| 删除 Collaboration leader | 关闭它会像如今一样停止其成员请求；成员仍是 Session |
| 删除成员 | leader 对它的请求像如今一样以成员已关闭结算 |
| 对同一源的两次分叉 | 两个独立的 Session，各自拥有共享的文件 |

**S6 评审意见。** "SessionStore 要先关闭 Session、放掉租约"：第 2 步在任何移除之前。"`attachment.put` 与删除竞争时要预期 `Locked` 或 `DirectoryNotEmpty`"：写入的目录不属于任何 Session（S5a），删除从不移除它，S5a 的清扫只移除无所有者且不在写入中的文件，因此两者都不会碰到写入的租约。命令输出的移除只会因已关闭 Session 之外的写入方遇到 `Locked` 或 `DirectoryNotEmpty`，这时删除失败并重试（§3.5）。"需要把 put 和 delete 串行化"：存储的写事务给它们排序（S5a §6），不另加内存锁。

## 8. 存储格式

**决定：不改格式。S5c 原样使用 S5a 的 v15。** 记录在
[native-storage-format.md](../../../design/appserver/native-storage-format.md#s5c-格式决定)。

- 删除只从现有表中删行。
- 分叉只向现有表插入行，按字节复制存储的文本。持久化 JSON 类型和事件格式样本不变。
- 事件 id 在所属 Session 内唯一，分叉在另一个 Session 中重复源的 id。存储本就按 Session 检查和索引 id；S5c 把这一点写明为不变量，不是格式变化。
- 不加来源列、不写墓碑、不加事件，原因见 §4 和 §6.6。
- 没有 S5c 的守护进程把分叉当作普通 Session 读取，把已删除的 Session 当作从未存在。

## 9. 归属与 S2

| 操作 | 所有者 | 调用方 |
| --- | --- | --- |
| `session.delete`、`session.fork`（参数格式、协议版本） | Rpc 内建方法 | 客户端 |
| 删除的编排：准入门、关闭、忘记 | Agents（`AgentIndex::delete`） | Rpc |
| 从另一个 Session 的切点创建 | Agents（`AgentIndex::fork`），即总体设计 B.2 中 Agents 的补充 | Rpc |
| 命令输出移除、行删除、分叉事务 | SessionStore 的 Rust 方法 | Agents |
| Session 文件的行与清扫 | SessionStore（S5a） | `sessionFiles.put`、`delete_session` |

**S2（[#62](https://github.com/arcships/dim/issues/62)）。** S2 的 `sessions.launch` 由 profile 和以启动方为上限的策略创建 Session，不从切点创建。Agents 为两者保留同一条创建路径：预备创建，`session.create` 和 `launch` 的预备步骤是 `create_owned_session`，分叉的预备步骤是分叉事务。S2 不为 `sessions` 加分叉方法。以后若加 `sessions.fork`，它调用 `AgentIndex::fork`，并像 `launch` 那样以启动方为上限限制复制来的策略，因为进程调用方不能得到比启动方更宽的预设。

## 10. 权限（ADR 003）

- 分叉复制权限预设，从不放宽。切点处的预设是源曾持有的，调用方本来就能用 `session.permission.update` 修改源的预设。
- 复制来的审批事实是对已结算调用的决定。Approval 每次回答都复查当前策略，不记录长期授权，因此它们不带来任何权限。
- 宿主安全设置从不进入 Timeline，分叉既不恢复也不放宽它们（ADR 003 D5）。
- 拥有源的 Session 文件不增加任何权限（[Session 文件 §11](session-files.md#11-安全)）。
- 复制来的对其他 Session 资源的引用（成员 Session id、任务 id、路径）不赋予任何权限：结算分叉的所有者只处理自己 Session 持有的资源（§14，Collaboration）。
- 删除不改变宿主状态；诊断只带 id 和错误类别，从不带事件内容。

## 11. 资源节点（P3）

S5c 与 S5a 一样只实现控制节点。在控制节点上，FileStorage 正在重载或已关闭时第 3 步失败，删除随后重试。在 P3 的资源节点上，第 3 步在该节点的 FileStorage 上执行，节点不可用时删除失败于节点不可用。删除能否照常提交、把目录留给该节点自行对账，在 P3 中随按资源节点查找（[#38](https://github.com/arcships/dim/issues/38)）一起决定：一个资源节点只服务一个控制节点，所以在那里列目录能确定条目的归属，这与共用的数据库目录不同。

## 12. 对其他文档的修改

| 文档 | 本 PR 中的修改 |
| --- | --- |
| [总体设计 B.2](../overview.md#b2-app-server-进程插件) | Rpc 行：内建方法 `session.delete` 与 `session.fork`。Agents 与 SessionStore 行链接到本页 |
| [原生存储格式](../../../design/appserver/native-storage-format.md#s5c-格式决定) | S5c 格式决定：不改格式 |
| [AcpGateway](../plugins/AcpGateway.md)、[RemoteGateway](../plugins/RemoteGateway.md) | 原文说分叉时重置预设，#26 已改为：预设随事实一起复制 |
| [文档索引](../../../INDEX.md) | 链接到本页 |
| [Session 文件](session-files.md) | §5.3 和 §12 原说节点离线时删除照样提交。这对其中的 Session 文件步骤成立；整个删除还会先在节点上移除命令输出（§11）。§5.2、§5.4 和 §6 不变，就是本页执行的步骤 |

## 13. 规则检查

| 规则 | 回答 |
| --- | --- |
| 原则 §4 第 1 条（改别人的状态写指令） | 删除和分叉只改 Agents 与 SessionStore 自己的状态：Session 记录和行。分叉复制存储的字节，不替其他所有者写事实；F 打开时写出的事实来自各自的所有者（TimelineTree 的 `branch/resume`、Loop 的终态）。预设是复制，不是重置（#26） |
| 原则 §4 第 2 条（交接） | 不适用：没有交接任何东西 |
| 原则 §4 第 3 条（before 只做变换） | 没有 before 回调。Rpc 检查参数格式和 Session 的版本；切点检查沿用 `session.branch_create` 现有的检查 |
| 原则 §4 第 4 条（核心不带业务含义） | Timeline、Dispatcher、before 和批次不变。TimelineTree 现有的规则（有事件的根分支以 resume 打开）原样适用 |
| 原则 §4 第 5 条（批次） | 没有事件。切点位于原子批次末尾，复制的都是完整批次 |
| 原则 §4 第 6 条（上下文只来自交接后的事件） | F 的上下文来自复制来的已采纳输入和工具结果，没有其他东西进入 |
| 原则 §4 第 7 条（只通过公开事件和服务） | Agents 使用 Session 门面（`close`、`context`）和 SessionStore 的方法；SessionStore 使用 FileStorage 公开的 `remove`。对 Collaboration 的需求让分叉不会作用于源的成员 |
| 原则 §4 第 8 条（向机制所有者提需求） | 不改 Timeline、Dispatcher、事件格式、装配、rutis 或 Session 门面，也不向 rutis 提需求。同步移除是 FileStorage 关于自身操作持久性的补充；Collaboration 的检查是它自己的规则（§14） |
| 原则 §4 第 9 条（只有后续决定要用的事实进 Timeline） | 没有 Timeline 事件。删除和分叉是对记录的进程级操作；没有任何决定使用来源，所以不存储 |
| 原则 §1（Capability 与 App Server 的边界） | 不新增插件；Agents 和 SessionStore 的补充已列在总体设计 B.2 |
| ADR 002 D1（作用域随状态） | 进程作用域：记录和数据库目录比任何 Session 都长寿 |
| ADR 002 D2（不设接线模块） | Rpc 内建方法调用 Agents；SessionStore 的步骤是内部方法，没有适配器或注册表 |
| ADR 002 D3（依赖随作用域） | Rpc、Agents 和 SessionStore 不依赖任何可选插件；FileStorage 每次调用时查找 |
| ADR 002 D4（结构配置与业务配置分离） | 没有配置 |
| ADR 002 D5（语言中立合同） | 两个线上方法只收发数据。SessionStore 不新增服务 |
| ADR 003（审批范围） | §10：分叉复制预设，从不放宽；复制来的事实和共享的文件不赋予权限 |

## 14. 向机制所有者提出的需求

不向运行时或 rutis 提需求。对其他所有者的补充与需求：

| 对象 | 需求 | 状态 |
| --- | --- | --- |
| FileStorage | `remove` 在报告成功之前同步被移除条目的父目录，同步失败时返回 `Unsynced`。Windows 没有目录同步，与 `replace` 一样依赖 NTFS 元数据日志。用于让删除的移除在提交之前落盘（§4） | 在 S5c 的实现中补充，与 S5a 中的 `create_directory` 一样；需与 FileStorage 的所有者确认 |
| Collaboration | 分支重建时，已送达但没有结果的请求以失败结算；只有成员关系指明本 Session 是 leader 时才停止该成员。一般地，重载结算只处理本 Session 持有的资源。如今重建会停止事实中指名的任何已加载 Session（[collaboration.rs](../../../../crates/dim-app-server/src/collaboration.rs) 的 `rebuild`），因此分叉一个 leader 会停止源的成员 | `session.fork` 上线前必须完成；验收第 6 条 |
| Goal（P5，[#69](https://github.com/arcships/dim/issues/69)） | [Goal](../plugins/Goal.md) 的 A11 假设复制来的事实之后有 `branch/start`。分叉的 `main` 以 `branch/resume` 打开，复制来的前缀含有源的 `branch/start` 和目标块。Goal 采用 A11 的回退做法，在分叉首次加载时清除继承来的生效目标（这是一次创建性构建且 `main` 已有事件：`SessionIdentity.restored` 为 false），或修订 A11。分叉提交之后、清除之前崩溃，目标仍会生效；要有持久的边界，需要 Session 访问 A.3 推迟的来源记录 | P5 的 Goal 设计评审 |
| S5b（[#60](https://github.com/arcships/dim/issues/60)） | 标题元数据行在删除事务中删除，分叉不复制 | S5b 的设计 |
| [#32](https://github.com/arcships/dim/issues/32) | `submission_id` 移出 `input/message` 后，分叉在其事务中复制切点及之前输入的幂等记录，让 `session.input_lookup` 在分叉上继续能回答复制来的输入 | #32 的设计 |
| F6（[#77](https://github.com/arcships/dim/issues/77)） | `dimcode session delete`，以及按时间或数量清理并预览，由 `session.summaries` 和 `session.delete` 组合（§5.1） | F6 的设计 |

## 15. 验收（流程测试）

1. 删除一个正在运行轮次、并有客户端附着的已加载 Session：客户端收到 `session.closed`，回复为 `{}`，`session.list` 不再列出它，`session.attach` 失败于 `session_closed`。任何表中都不再有它 id 的行，`command-output/<id>/` 已移除，它的 Session 文件由清扫移除。第二次删除回复 `{}`。
2. 删除未加载的 Session，以及行校验失败的 Session：都回复 `{}`，不留任何行。
3. 在 `main` 的上下文边界处分叉，一个子分支在切点之前创建、一个在之后：F 的 `main` 含有源截至 `at` 的事件且 id 相同，之后是 `branch/resume`；F 整体含有第一个子分支，不含第二个；F 只有其事件引用的内容行，重启后能重新加载。
4. 分叉的 `SessionConfig` 文本与源相同，包括 `"interactive": false`。切点之前在源上设置的权限预设和模型就是 F 的预设和模型。
5. `at` 超过头部、位于批次内部或不在上下文边界，源未知，以及 v2 连接：分叉都被拒绝，什么都没创建。
6. 在 leader 对成员的请求未结束时分叉：F 以失败结算该请求，源的成员继续运行。
7. 分叉后删除源：F 能加载并读取它的 Session 文件；删除 F 后文件被移除。对同一源的删除与分叉竞争：要么 F 带着文件存在，要么分叉失败于 `session_closed`；绝不会出现没有文件的分叉。
8. 在一次性存储上：命令输出目录已不在但行还在的 Session 能加载，随后删除能完成；删除提交后留下的无所有者文件行在下一代的第一次写入时被清扫。

验证：`./dev check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` 与 `cargo test --workspace --locked`。不改 Capability、装配或 crate 依赖，`xtask` 只需按常规在 CI 中运行。

## 附录 A. Rust 绑定

在 dim-app-server 中：

```rust
impl AgentIndex {
    /// `session.delete`: gate, close if loaded, `SessionStore::delete_session`, forget (§3.2).
    pub(crate) async fn delete(&self, id: &str) -> dim_agent_runtime::Result<()>;
    /// `session.fork`: acquire the source, check the cut, prepared creation (§6.5).
    pub(crate) async fn fork(&self, source: &str, at: u64) -> dim_agent_runtime::Result<Session>;
}
impl SessionStore {
    /// Removes `command-output/<id>` (synced), then every row in one transaction;
    /// schedules the Session-file sweep.
    pub async fn delete_session(&self, id: &str) -> Result<()>;
    /// Prepare step of a fork: copies the rows of §6.3 in one transaction.
    pub async fn fork_session(&self, source: &str, fork: &str, at: u64) -> Result<()>;
}
```

`SessionStorePlugin` 已经接收数据库目录的根索引（S5a）。它在该根下移除 `command-output/<id>`，这就是守护进程交给 WorkspaceTools 作为输出根的目录（[main.rs](../../../../crates/dim-app-server/src/main.rs)）；这个名字只定义一次，两处共用。创建回滚路径（`TimelineStore::rollback_session`）调用与 `delete_session` 相同的行删除。
