# Scheduler

状态：草案。总体设计：[overview B.8](../overview.md)；作用域：[ADR 002](../../adr/002-plugin-scope.md)（进程 + Session）。阶段：P2。语言：优先 TypeScript（overview §F.3）。

Scheduler 把定时任务及其运行历史保存在 App Server 进程存储中。守护进程存活且某个窗口到期时，进程部分通过 Agents `launch` 创建一个非交互 Session（或通过 `AgentIndex` 复用一个），并通过 Session 公共接口（[session-access](../contracts/session-access.md)）把任务提示词写成带自身来源标记的 Queue 输入，方式与 Collaboration 访问其成员相同。Session 部分向交互式 leader Session 提供 `schedule` 工具（风险 `persistent_configuration`）。运维方使用管理级 `schedule.*` 方法。Scheduler 不写自己的 Timeline 事件：一次运行的结果从运行 Session 的历史中读取，并作为运行结果事件通知发布。

## 1. 结构

```mermaid
flowchart LR
  subgraph PROC["进程（控制节点）"]
    Sched["Scheduler：定时器、运行跟踪器、任务存储"]
    MM["ManagementMethods：schedule.*"]
    Out["运行结果事件通知（EventKey）"]
    Agents["Agents：launch、AgentIndex"]
    Rpc["Rpc 方法表"]
    Store["进程存储（App Server）"]
    Listen["监听方，如 Notifications（无依赖）"]
  end
  subgraph SESS["交互式 leader Session"]
    Tool["SchedulerSession：ToolContribution schedule"]
  end
  subgraph RUN["运行 Session（非交互）"]
    TL["main Timeline：input/message、input/consumed、turn/end"]
  end
  Tool -->|"外层作用域"| Sched
  Sched --> Agents
  Sched --> Store
  Sched --> MM
  Rpc -->|"ManagementMethods 的 Contributions"| MM
  Agents -->|"launch、acquire、Session 公共接口"| TL
  Sched --> Out
  Listen -.-> Out
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
  class Sched,MM,Out,Tool new
```

任务和窗口跨越多个 Session，因此属于进程（[ADR 002](../../adr/002-plugin-scope.md) D1）；向 Session 提供工具是 Session 部分的职责（D2）。Session 部分依赖进程部分（外层作用域）。进程部分依赖 Agents 和 App Server 进程存储；它只通过 `AgentIndex` 和 Session 公共接口访问运行 Session。依赖遵循作用域（D3）：Agents 重载会重载 Scheduler，Scheduler 重载会重载其 Session 部分，使它们的在途轮次失败；两者都是少见的结构性事件，属于已接受并有文档记录的代价。任务编辑属于业务配置（D4）：数据更新加通知，从不重载。

| 部分 | 作用域 | 依赖 | 提供 |
| --- | --- | --- | --- |
| `Scheduler` | 进程（控制节点） | Agents（带 `policy` 的 `launch`、`AgentIndex`）、App Server 进程存储 | 任务存储、定时器、运行跟踪器；由 Rpc 收集的 `ManagementMethods` 条目 `schedule.*`；事件 key `scheduled_run_outcomes` |
| `SchedulerSession` | Session，Scheduler 启用时安装 | `Scheduler`、`S:SessionConfig` | Session 级 `ToolContribution` `schedule`，仅在 `interactive` 且不是 Collaboration 成员时贡献 |

**依赖的契约**：[session-access](../contracts/session-access.md)（带受启动方上限约束的 `policy` 的 Agents `launch`、`SessionConfig` 字段 `interactive`/`resource_node`/`created_by`、带来源 raw 块的输入、固定前缀读取、撤回）、Session 级 `ToolContribution`（[工具贡献](../../../architecture/rust-agent/tool-contributions.md)）、Rpc 方法表中的管理级方法（[session-boundary](../contracts/session-boundary.md)）、Permission 类别 `persistent_configuration`（FullAccess 允许，E1）、App Server 进程存储（R8）、[单树事件契约](../../../architecture/rust-agent/single-tree.md)。**本文定义的契约**：任务与运行格式、`schedule` 工具、`schedule.*` 方法、运行结果通知（A.6）、来源名 `scheduler`（标记 `dim-scheduler`，提交 id `scheduler:<run_id>`）。

## 2. 数据与状态

```mermaid
classDiagram
  class Job {
    id, name, schedule, prompt
    cwd, resource_node, model, agent_id
    mode create 或 reuse, session_id
    policy, policy_cap
    enabled, max_run_minutes, revision, created_by
  }
  class Run {
    run_id, job_id, window, state
    session_id, input_event_id, submission_id
    outcome, detail, 时间戳
  }
  class ProcessStorage {
    scheduler 根目录, 格式版本 1
    原子替换, 锁
  }
  class RunSessionTimeline {
    带 dim-scheduler 标记的 input/message
    input/applied, input/consumed, input/withdrawn
    turn/start, turn/end, stop/interrupt
  }
  Job "1" --> "0..100" Run
  ProcessStorage *-- Job
  ProcessStorage *-- Run
  Run ..> RunSessionTimeline : 由历史判定
```

任务和运行只存在于进程存储中；不从 Timeline 重建任何内容，Scheduler 不拥有 `StateCell`。运行进度由运行 Session `main` 分支上的已有事实推导。Session 部分不持有状态。

## 3. 流程

### 3.1 窗口到期

```mermaid
sequenceDiagram
  participant T as Scheduler 定时器
  participant ST as 进程存储
  participant AG as Agents
  participant S as 运行 Session（main）
  participant IQ as InputQueue 与 Loop
  T->>ST: 复查任务，持久化运行 launching
  alt mode create，或首次 reuse 运行
    T->>AG: launch(cwd, node, model, interactive false, policy)
  else reuse
    T->>AG: AgentIndex acquire(session_id)
  end
  AG-->>T: Session 使用（租约）
  T->>S: Queue 输入，标记 scheduled-run，用户提示词
  T->>ST: 持久化 submitted 及 input_event_id
  S->>IQ: Dispatcher：input/applied、input/consumed、轮次
```

运行在任何外部效果之前就以 `launching` 持久化，因此重启时通过查找而不是重新提交来结算它。

### 3.2 完成与超时

```mermaid
sequenceDiagram
  participant R as 运行跟踪器
  participant S as 运行 Session（main）
  participant ST as 进程存储
  R->>S: idle(main) 返回截止点 C
  R->>S: local_page 读到 C
  R->>R: 已消费？消费后的首个 turn/end？
  R->>ST: running 或终态结果
  R-->>R: 终态：发出 RunOutcome 通知
  Note over R: max_run_minutes 已到
  alt 输入未被消费
    R->>S: 撤回自己的输入
    R->>ST: input/withdrawn 之后记 timed_out
  else 已消费
    R->>S: stop(main, scheduled run timed out)
    R->>ST: 读到 turn/end 后记 timed_out
  end
```

每个活动运行都有一个任务，等待 `Session::idle` 并读取到返回截止点为止的历史，与 Collaboration 和 Workflow 的做法相同；Scheduler 不观察 Session 广播。若撤回时发现输入已被消费，跟踪器退回到 `stop`。通知只在终态记录持久化之后发出一次。

### 3.3 运行的权限

```mermaid
flowchart LR
  Created{"created_by"}
  Tool["工具：上限 = 保存时创建方 Session 的 policy"]
  User["管理：上限 = 宿主配置最大值"]
  Default1["默认 = 上限"]
  Default2["默认 = 只读"]
  Declared["任务 policy（可选，只能收窄）"]
  Eff["生效 = min(声明值或默认值, 上限)"]
  Created --> Tool --> Default1 --> Eff
  Created --> User --> Default2 --> Eff
  Declared --> Eff
```

自动化从不扩大权限（E2）：生效 policy 传入 `launch` 的 `policy`，Agents 按启动方（任务的 `policy_cap`）设置上限。复用的 Session 保留自己的 policy；若它超过任务的生效 policy，运行记为 `skipped: policy`。

## 4. 时序与冲突

```mermaid
sequenceDiagram
  participant S as Scheduler
  participant U as 用户
  participant IQ as InputQueue
  S->>IQ: 运行输入（Queue），序号 n
  U->>IQ: session.input.withdraw，序号 n+1
  IQ->>IQ: input/withdrawn
  S->>S: 下次读历史：已撤回，从未消费
  S->>S: 运行 cancelled（withdrawn）
```

所有关于待处理输入的判定都排除已撤回的输入，因此被撤回的运行输入不会让运行永远停在 `submitted`。

| 场景 | 序号顺序 | 结果 | 负责方 |
| --- | --- | --- | --- |
| 窗口触发时上一次运行仍在进行 | —（存储） | `skipped: overlap`（A4） | Scheduler |
| 用户中断运行的轮次 | consumed < `stop/interrupt` < `turn/end` | 运行 `cancelled` | Loop；Scheduler 记录 |
| 超时撤回与消费竞争 | consumed < withdraw | 撤回被拒；走 `stop` 路径 | InputQueue、Scheduler |
| Agents 或 Scheduler 重载、重启 | — | `launching` → 查找：已接受 → `submitted`，未知 → `lost`；从不重新提交；恢复跟踪 | Scheduler |
| 窗口期间守护进程停止 | — | `missed`；不补跑（E5） | Scheduler |

完整表格：附录 B。

## 5. 上下文映射

| 位置 | 模型看到的内容 | 形式 | 位置与存续 |
| --- | --- | --- | --- |
| 调用方 Session | `schedule` 声明 | 已采用配置中的工具声明（仅交互式 leader） | 配置；压缩后保留 |
| 调用方 Session | `schedule` 结果 | `ToolResult.content` | 批次结果位置；压缩后只存在于摘要中 |
| 运行 Session | `<dim-scheduler kind="scheduled-run" job="..." run="...">job name</dim-scheduler>`，随后是作为用户内容的提示词 | 一个 Queue 输入的 raw 标记块 + 用户文本 | 位于其 `input/consumed` 位置；不带 key，压缩后只经由摘要保留；若在切点前已被消费，子分支会继承 |

运行 Session 是非交互的：没有 `ask_user`，没有 `schedule` 工具。`reuse` Session 中之前的运行像任何输入一样保留为历史。模型从不看到任务记录、运行历史、定时器或诊断信息。

## 6. 接口（摘要）

| 接口 | 类型 | 用途 |
| --- | --- | --- |
| `schedule` | Session 级 `ToolContribution`，风险 `persistent_configuration` | 模型管理任务 |
| `schedule.list/save/delete/set_enabled/run_now/history` | 管理级方法（`ManagementMethods`） | 运维方管理任务 |
| `scheduled_run_outcomes()` | 本文定义的事件 key | 每次运行结束时的实时通知（Notifications 监听） |
| `/scheduled`、`/schedule`、`dimcode schedule` | CLI | 调用管理方法 |
| Agents `launch` | 依赖 | 带节点、模型和受启动方上限约束的 `policy` 的非交互运行 Session |

## 7. 安全

`schedule` 由 Approval 和 Permission 按 `persistent_configuration` 准入；FullAccess 允许（E1），其他预设询问或拒绝。运行只写入输入；其工具在运行 Session 的 policy 下准入，上限按 3.3 设定，审批不会预先授予。工具创建的任务记录 `created_by`，可供审查。任务不保存凭据。运行 Session 只使用自身资源节点的服务；Scheduler 不触碰任何服务。拥有管理访问权的人都能读取任务提示词。

## 附录 A. 接口细节

### A.1 记录

| 记录 | 字段 |
| --- | --- |
| Job | `id`、`name`、`schedule`（A.4）、`prompt`（文本 ≤ 32 KiB）、`cwd`、`resource_node`（`None` = 控制节点）、`model`（`None` = Agents 默认）、`agent_id`（可选 profile）、`mode`（`create` \| `reuse`）、`session_id`（`reuse` 在首次运行时绑定）、`policy`（可选预设，只能收窄）、`policy_cap`（保存时捕获）、`enabled`、`max_run_minutes`（默认 60，A7）、`revision`、`created_by`（`user` \| `tool:<session_id>`） |
| Run | `run_id`、`job_id`、`window`（时刻或 `manual`）、`state`、`session_id`、`input_event_id`、`submission_id`、时间戳、`outcome`（`completed` \| `failed` \| `cancelled` 及原因）、`detail` |

运行状态：`skipped`（overlap、capacity、disabled、node unavailable、policy）、`missed`、`launching`、`submitted`、`running`，终态 `completed`/`failed`/`cancelled`/`timed_out`/`lost`。历史为每个任务保留最近 100 次运行（A8）。写入在存储锁下以原子替换进行并检查 `revision`，因此工具保存和管理保存不会静默互相覆盖。

### A.2 工具 `schedule`

仅在 `SessionConfig` 为 `interactive: true` 且没有成员角色的 Session 中由 `SchedulerSession` 贡献（A9），因此运行 Session 和成员从不看到它，运行也不能再调度更多运行。处理器调用进程部分。所有动作的风险都是 `persistent_configuration`（A10）。

| 参数 | 类型 | 规则 |
| --- | --- | --- |
| `action` | `list` \| `get` \| `create` \| `update` \| `delete` \| `set_enabled` \| `run_now` \| `history` | 必填 |
| `job_id` | string | 除 `list`、`create` 外均需 |
| `job` | object：`name`、`schedule`、`prompt`、`cwd?`、`model?`、`mode?`、`policy?`、`max_run_minutes?` | `create`、`update`（部分字段） |
| `enabled` | bool | `set_enabled` |
| `limit` | integer ≤ 50 | `history` |

`cwd` 和 `resource_node` 默认取调用方 Session 的 `SessionConfig`；工具不能指向其他资源节点（A11）。在 `create`/`update` 时，`policy_cap` 为调用方 Session 当前的 policy，由 Agents 解析（A15）。结果 `content`：包含 `job_id`、下一个窗口或列表的简短文本；`structured_content` 保存记录。校验错误作为错误结果返回。

### A.3 管理方法

| 方法 | 参数 | 结果 |
| --- | --- | --- |
| `schedule.list` | `{}` | 任务及 `next_window`、最近一次运行 |
| `schedule.save` | `{job, revision?}` | 保存后的任务（`policy_cap` = 宿主配置最大值）；过期 `revision` → `schedule_conflict` |
| `schedule.delete` | `{job_id}` | 删除任务及其历史；活动运行不会被停止（A14） |
| `schedule.set_enabled` | `{job_id, enabled}` | 暂停/恢复 |
| `schedule.run_now` | `{job_id}` | `run_id` 或跳过原因 |
| `schedule.history` | `{job_id, limit?}` | 运行，最新在前 |

Scheduler 重载期间，Rpc 回答 `service_reloading`；未启用时回答 `unknown_method`。

### A.4 调度格式

| 类型 | 形式 | 规则 |
| --- | --- | --- |
| `cron` | `{cron: "m h dom mon dow", tz?: IANA}` | 五个字段，支持列表、范围、步长；`tz` 默认取控制节点时区；夏令时空缺时刻跳过，重复时刻只触发一次 |
| `every` | `{every_minutes: n}` | `n` ≥ 1；窗口位于 `anchor + k·n`，anchor = 最近一次启用时间 |
| `at` | `{at: RFC 3339 instant}` | 单个窗口；触发或错过后任务被禁用 |

粒度为一分钟（A1）。窗口只有在其时刻后 60 秒内被观察到才会触发（A2）；否则用一条 `missed` 记录合并整段时间。定时器在每次 tick 和唤醒时评估墙钟时间，因此挂起的时间从不补跑。

### A.5 Timeline 与 Capability 机制

| 步骤 | 路径 |
| --- | --- |
| 触发 | 定时器任务（`ctx.effect`）为每个启用的任务计算下一个窗口；到达窗口时应用 overlap、capacity（A13）、启用状态和 policy 规则；用新的 `run_id` 持久化 `launching` |
| Session | `create`：Agents `launch { interactive: false, resource_node, model, agent_id, policy }`（`created_by: Launch { source: "scheduler" }`）。`reuse`：`AgentIndex::acquire(session_id)`（未加载时恢复）；首次运行 launch 并绑定 `session_id` |
| 写入 | 在 `main` 上 `Session::input`：`submission_id: "scheduler:<run_id>"`，标记 `{kind: "scheduled-run", attrs: [job, run], body: job name}`，用户提示词，模式 Queue → `Timeline::append`；持久化 `submitted` |
| 消费 | InputQueue 追加 `input/applied`；Loop 追加 `input/consumed { input_id }` 并驱动轮次 |
| 完成 | 跟踪器任务：`Session::idle("main")` → 截止点 `C`；`local_page` 读到 `C`；一旦存在 `input/consumed { input_id = input_event_id }` 即为 `running`；在其后的首个 `turn/end` 结束；消费前出现 `input/withdrawn` → `cancelled: withdrawn` |
| 超时 | 未消费：撤回自己的输入 → `timed_out`；已消费：`Session::stop("main", ...)`（轮次作用域），读到 `turn/end` 时记 `timed_out` |
| 复查 | 写入前从存储重新读取任务和运行（已启用、revision 未变、运行仍为 `launching`） |
| 重载或重启 | Session 部分随 Scheduler 重载；在途的 `schedule` 调用随其轮次失败。新一代进行结算：`launching` → 按 `submission_id` 查找：已接受 → `submitted`，未知 → `lost`（从不重新提交）；`submitted`/`running` 恢复跟踪；已过去的窗口记 `missed` |
| 运行结果 | 持久化终态后执行 `emit(host_ctx, &scheduled_run_outcomes(), Arc::new(outcome))`；重载或重启后不重放任何内容 |

### A.6 运行结果通知（公共契约）

```rust
/// Process event key defined in Scheduler's public types crate; listeners
/// register with `on(host_ctx, &key, listener)` and form no dependency on Scheduler.
pub fn scheduled_run_outcomes() -> EventKey<RunOutcome>;
pub struct RunOutcome {
    pub job_id: String, pub job_name: String, pub run_id: String,
    pub session_id: Option<String>,
    pub state: RunEnd,               // Completed | Failed | Cancelled | TimedOut | Lost
    pub reason: Option<String>,      // fixed reason text, never Session content
    pub ended_at: Timestamp,
}
```

每次运行在终态记录持久化之后发出一次（A16、A17）。投递是实时的、不可续传；未加载的监听方会错过通知。通知不携带提示词或 Session 内容；`schedule.history` 是持久记录。

## 附录 B. 完整冲突表

| 场景 | 序号顺序 | 结果 | 负责方 |
| --- | --- | --- | --- |
| 窗口触发时上一次运行仍在进行 | — | `skipped: overlap`；不写输入（A4） | Scheduler |
| 运行进行中时调用 `run_now` | — | 拒绝，`skipped: overlap` | Scheduler |
| 复用的 Session 正忙 | 先前事实 < 运行输入 | 以 Queue 写入；在当前轮次之后被消费（A5） | InputQueue、Loop |
| 复用的 Session：用户并发 Steer/Queue/Append | Timeline 顺序 | 按序号顺序依模式应用；运行按自己的 `input_id` 跟踪 | InputQueue |
| 用户中断运行的轮次 | consumed < interrupt < `turn/end` | `cancelled` | Loop |
| 中断发生在运行输入之前 | interrupt < input | 输入启动一个轮次 | Loop |
| 输入已应用，消费前中断 | applied < interrupt | 保持待处理；由下一轮次消费；运行 `submitted` | Loop |
| 用户撤回运行输入 | input < `input/withdrawn` | 运行 `cancelled: withdrawn` | InputQueue、Scheduler |
| 超时撤回与消费竞争 | consumed < withdraw 请求 | 撤回被拒；`stop` 后 `timed_out` | InputQueue、Scheduler |
| 写入前分支关闭或 Session 已关闭 | — | `failed: session_missing`；保留 reuse 绑定（A6） | Scheduler |
| 运行进行中 Session 被关闭 | — | 历史读取失败；`failed` | Scheduler |
| 运行期间发生压缩 | — | 无影响：`input/consumed` 与 `turn/end` 仍在本地历史中 | — |
| 运行 Session 中创建子分支 | — | 忽略；运行只使用 `main` | Scheduler |
| 运行 Session 在输入待处理时因空闲卸载 | — | 跟踪器的租约使其保持加载；重启后由 `acquire` 恢复 | AgentIndex |
| 运行中 Agents 重载 | — | Scheduler 随之重载；运行保留；从历史恢复跟踪 | Scheduler |
| 运行中 Scheduler 重载或重启 | — | 按 A.5 结算；从不重新提交 | Scheduler |
| `schedule` 调用期间 Scheduler 重载 | — | 调用方 Session 部分重载；该轮次失败（D3，已接受） | Loop |
| 窗口期间守护进程停止 | — | `missed`；不补跑 | Scheduler |
| 触发时资源节点离线，`create` | — | `launch` 在启动期限到达时失败 → `skipped: node_unavailable`；不重试（A6） | Agents、Scheduler |
| 资源节点离线，`reuse` | — | 输入提交；恢复后开始轮次；超时适用 | rutis 生命周期、Scheduler |
| 复用 Session 的 policy 高于任务的生效 policy | — | `skipped: policy` | Scheduler |
| 运行 Session 中有待处理审批 | `approval/request` | 等待任意客户端的 `approval.respond` 或超时 | Approval；Scheduler |
| 重复/迟到的 `schedule.save` | revision | 过期 revision 被拒 | Scheduler |
| 保存后 `schedule` 调用被中断 | save < interrupt | 已保存的任务保留；工具结果取消 | ToolExecutor；Scheduler |
| 客户端断开 | — | 无影响 | — |

## 附录 C. 错误与恢复

| 情况 | 结果 |
| --- | --- |
| 无效任务（调度、提示词大小、未知节点、policy 超过上限） | `schedule_invalid` 或错误工具结果 |
| launch 失败 | 运行 `failed`；窗口不重试 |
| 写入不确定（重载、发布未知） | 运行保持 `launching`；通过查找结算 |
| 存储不可读或格式版本未知 | Scheduler apply 失败（不静默重置） |
| 进程退出 | 定时器停止；下次启动时结算 |

错误码：`schedule_invalid`、`schedule_conflict`、`schedule_not_found`、`service_reloading`。

## 附录 D. 验收

使用可控时钟，通过管理方法、`schedule` 工具和通知：

1. 每种调度类型产生预期窗口，包括夏令时空缺/重叠以及 `at` 自行禁用。
2. 到期的 `create` 任务产生一个非交互 Session，其首个输入携带标记和提示词；运行 `completed`；一条通知。
3. 重叠：活动运行期间的窗口记为 `skipped: overlap`。
4. `reuse` 且 Session 正忙：输入排在当前轮次之后。
5. 用户中断 → `cancelled`；消费前超时 → `input/withdrawn` 与 `timed_out`；消费后超时 → `stop/interrupt` 与 `timed_out`。
6. 用户撤回运行输入 → `cancelled: withdrawn`。
7. 时钟跳变 → 一条 `missed` 记录，不运行。
8. 存在 `launching` 运行时重启：已接受的恢复，未知的 → `lost`；无重复输入。
9. 运行中 Agents 重载：Scheduler 重载，运行完成并被跟踪到结束。
10. Policy：在 WorkspaceWrite 下由工具创建的任务至多以 WorkspaceWrite 运行；管理创建的任务默认 ReadOnly。
11. 非交互 Session 和成员 Session 中没有 `schedule`；WorkspaceWrite 下询问，FullAccess 下允许。
12. 触发时节点不可用：`skipped: node_unavailable`。

## 附录 E. 假设

| # | 假设 | 依据 | 若被否决的影响 |
| --- | --- | --- | --- |
| A1 | 格式 `cron`、`every`、`at`；一分钟粒度 | 功能清单（"recurrence"）；判断 | 格式变化 |
| A2 | 窗口宽限 60 秒 | 判断 | 其他 missed 检测方式 |
| A4 | 重叠：跳过新窗口 | 判断 | 排队或并行运行 |
| A5 | 复用的忙碌 Session：入队 | 判断 | 忙碌时的窗口被跳过 |
| A6 | 失败从不重试，也不退回到 `create` | 判断 | 回退策略 |
| A7 | `max_run_minutes` 默认 60 | 判断 | 运行无上限 |
| A8 | 每个任务保留最近 100 次运行 | 判断 | 保留期设置 |
| A9 | 工具只给交互式 leader Session | Overview §5.1；判断 | 运行可创建任务 |
| A10 | 所有动作使用同一风险类别 | Overview §5.8 | 拆出只读工具 |
| A11 | 工具创建的任务使用调用方的资源节点 | 判断（overview §6.3） | 允许其他节点 |
| A13 | 最大并发运行数 = 4，属于业务设置，在每个窗口读取 | 判断；ADR 002 D4 | 其他上限 |
| A14 | 删除任务不停止其活动运行 | 判断 | 删除即停止运行 |
| A15 | 工具创建任务的上限在保存时捕获 | E2 范围内的判断 | 每次运行重新解析 |
| A16 | 运行结果是实时、不可续传的通知 | Overview §5.11；判断 | 每个监听方的持久发件箱 |
| A17 | 只有到达 `launching` 的运行才产生通知 | 判断 | 监听方看到每个窗口 |

已决定：运行 policy（E2）、FullAccess 允许 `persistent_configuration`（E1）、守护进程存活（E5）、进程存储（R8）、撤回（R7）、作用域（[ADR 002](../../adr/002-plugin-scope.md)）。

## 附录 F. 待对齐问题

| 问题 | 说明 |
| --- | --- |
| 存活表述 | 功能清单称交互终端必须存活；本设计保留"守护进程存活"（E5）；已记录，不再开放 |
| Launch 契约 | Agents 补充必须定义带受启动方上限约束的 `policy` 的 `launch`，以及解析 Session 当前 policy 的方式 |
