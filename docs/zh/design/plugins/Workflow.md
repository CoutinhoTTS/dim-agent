# Workflow

状态：草案。总体设计：[overview §B.9](../overview.md)；作用域：[ADR 002](../../adr/002-plugin-scope.md)（进程 + Session）。阶段：P2。语言：优先 TypeScript（[overview §F.3](../overview.md)）。

Workflow 拥有工作流定义和运行。一次运行推进一个固定的步骤列表；每个阶段在自己的 Session 中运行，要么是通过 Agents 启动的新非交互 Session，要么是用 Collaboration 的 `establish` 创建的、起始 Session 的 Collaboration 成员。进程部分拥有运行，并通过进程服务 `sessions`（[session-access](../contracts/session-access.md)）访问 Session，方式与 Collaboration 相同。Session 部分以 `toolContribution` 向交互 Session 提供 `workflow` 工具；运维方使用管理级 `workflow.*` 方法，即一个 `managementMethods` 条目。所有契约都是语言中立的命名服务（[ADR 002 D5](../../adr/002-plugin-scope.md)）；Rust 形式见[附录 G](#附录-g-rust-绑定)。运行结束时，其结果以 Steer 输入向起始分支报告一次。Workflow 不写自己的 Timeline 事件。

## 1. 结构

```mermaid
flowchart LR
  subgraph PROC["进程（控制节点）"]
    WF["Workflow：定义、运行引擎、阶段执行器"]
    MM["managementMethods key workflow"]
    Agents["Agents：sessions"]
    Coll["Collaboration：establish"]
    Prov["Provider：purpose workflow.stage"]
    Set["Settings：用户与项目来源、宿主本地批准"]
    PC["PluginContents：类型 workflows"]
    Store["进程存储（App Server）"]
    Rpc["Rpc 方法表"]
  end
  subgraph SESS["交互 Session（起始方）"]
    Tool["WorkflowSession：toolContribution key workflow"]
  end
  subgraph STG["阶段 Session"]
    TL["main Timeline"]
  end
  Tool -->|"外层作用域"| WF
  WF --> Agents
  WF --> Coll
  WF --> Prov
  WF --> Set
  WF -.->|"可选"| PC
  WF --> Store
  WF --> MM
  Rpc -->|"按名称收集 managementMethods"| MM
  Agents -->|"sessions 调用"| TL
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
  class WF,MM,Tool new
```

一次运行跨越多个 Session，因此运行属于进程（[ADR 002](../../adr/002-plugin-scope.md) D1）；工具由 Session 级 `toolContribution` 提供（D2）。Session 部分依赖进程部分；进程部分依赖同一作用域的 Agents、Collaboration、Provider 和 Settings。依赖遵循作用域（D3）：Agents 或 Collaboration 重载会重载 Workflow，后者将其运行结算为 `interrupted`（A.6）；Workflow 重载会重载其 Session 部分，并使其在途轮次失败。这些都是罕见的结构性事件，属于已接受并有文档记录的代价。定义编辑和审批属于业务配置（D4）：在使用时读取，绝不重载。

| 部分 | 作用域 | 依赖 | 提供 |
| --- | --- | --- | --- |
| `Workflow` | 进程（控制节点） | `sessions`（Agents；`launch` 带 `policy`）、Collaboration（`establish`）、Provider 模型选择、Settings 用户/项目来源及宿主本地批准服务、`PluginContents`（可选，类型 `workflows`）、App Server 进程存储 | 运行引擎；`managementMethods` key `workflow` |
| `WorkflowSession` | Session，Workflow 启用时安装 | `Workflow`、Session 配置 | `toolContribution` key `workflow`，仅在 `interactive` 时声明 |

**依赖的契约**：`sessions`（[session-access](../contracts/session-access.md)：带启动方上限 `policy` 的 `launch`、Session 配置字段、带来源 raw 块的 `input`、`history`、`idle`、`stop`、`withdraw`）、Collaboration `establish`（Collaboration 补充，A.5）、`toolContribution`（ToolRegistry 接受命名工具服务，属于补充；[工具贡献](../../../architecture/rust-agent/tool-contributions.md)）、由 Rpc 收集的 `managementMethods`（[session-boundary](../contracts/session-boundary.md)）；以及现有的类型化服务：Provider purpose 策略、App Server 进程存储（R8）、Settings 来源和宿主本地批准服务（R11），待 [#4](https://github.com/arcships/dim/issues/4) 暴露后 TypeScript 部分可按名称访问。**此处定义的契约**：定义格式 `dim.workflow/v1`、`workflow` 工具、`workflow.*` 方法、来源名 `workflow`（标记类型 `stage` 和 `report`）、purpose key `workflow.stage`、内容类型 key `workflows`、运行记录。

## 2. 数据与状态

```mermaid
classDiagram
  class Definition {
    schema dim.workflow/v1
    name, description, parameters
    steps: stage or parallel group
    policy optional
  }
  class Run {
    run_id, name, source, fingerprint, snapshot
    params, cwd, resource_node, policy_cap
    report_to, started_by, state
    report: none, pending, delivered, undeliverable
  }
  class Stage {
    id, step, state, runner
    session_id, agent_id, submission_id
    input_event_id, result_text, reason
  }
  class ProcessStorage {
    workflow root, format version 1
  }
  class StageSessionTimeline {
    stage input, input/consumed, input/withdrawn
    turn/end, stop/interrupt
  }
  class StarterTimeline {
    workflow tool call and result
    report Steer input
  }
  Definition <.. Run : 启动时快照
  Run "1" *-- "1..*" Stage
  ProcessStorage *-- Run
  Stage ..> StageSessionTimeline : 由历史读取判定
  Run ..> StarterTimeline : 报告只写一次
```

定义在每次调用时从其来源读取（A1），审批在每次检查时读取；运行存于进程存储，并在每次状态转换时、在其所守护的副作用之前持久化。阶段进度由每个阶段 Session 中的已有事实推导；只复制有界的结果文本。不从 Timeline 重建任何内容；Workflow 不拥有 `StateCell`，Session 部分不持有状态。

## 3. 流程

### 3.1 从工具启动并运行一个阶段

```mermaid
sequenceDiagram
  participant M as Model (starter branch)
  participant WF as Workflow
  participant ST as Process storage
  participant AG as sessions.launch or Collaboration establish
  participant SS as Stage Session
  M->>WF: workflow start（WorkflowSession 的工具处理器）
  WF->>WF: 校验、审批检查、取起始方 policy 上限
  WF->>ST: 持久化 Run running
  WF-->>M: ToolResult run_id（批次结果）
  WF->>ST: 持久化阶段 launching
  WF->>AG: launch（非交互，受上限 policy）或 establish（leader, agent）
  AG-->>WF: sessionId
  WF->>SS: sessions.input main Queue，标记 stage，渲染后的提示词
  WF->>ST: 持久化阶段 running 及 input_event_id
  SS->>SS: Dispatcher：input/applied、input/consumed、轮次
  WF->>SS: sessions.idle(main)，然后 sessions.history 直到截止点
  WF->>ST: 阶段 completed 或 failed，下一步
```

处理器持久化运行后立即返回；阶段推进在进程部分的任务跟踪器上运行。完成状态只根据阶段 Session 中直到 `idle` 截止点的事实判定。

### 3.2 报告与停止

```mermaid
sequenceDiagram
  participant WF as Workflow
  participant ST as Process storage
  participant STB as Starter branch
  alt 运行 completed、failed 或 interrupted，且有 report_to
    WF->>ST: 报告 pending(submission_id)
    WF->>STB: sessions.input Steer，标记 report
    WF->>ST: delivered；分支缺失或 Session 关闭时 undeliverable
  else workflow stop
    WF->>ST: stopping
    WF->>WF: sessions.withdraw 自己未消费的阶段输入，对运行中阶段 sessions.stop(main)
    WF->>ST: stopped，无报告
  end
```

所有者显式停止绝不唤醒起始方（[event-driven flows §3](../../../architecture/rust-agent/event-driven-flows.md)）。任何重试前按 `submissionId` 执行 `sessions.lookup`，保证至多一份报告。

### 3.3 阶段的 policy

```mermaid
flowchart LR
  Start{"started_by"}
  ToolStart["tool：上限 = 起始方 Session 的 policy（由 Agents 解析）"]
  MgmtStart["management：上限 = 宿主配置最大值"]
  D1["默认 = 上限"]
  D2["默认 = 只读"]
  Decl["定义 policy（可选，只收窄）"]
  Eff["阶段 policy = min(声明值或默认值, 上限)"]
  Member["member 执行器：Collaboration leader-policy 规则，同样受上限"]
  Start --> ToolStart --> D1 --> Eff
  Start --> MgmtStart --> D2 --> Eff
  Decl --> Eff
  Eff --> Member
```

运行绝不扩大权限（E2）：`workflow` 工具按与 Collaboration `agent` 工具相同的风险类别准入，阶段 Session 绝不超过起始方的 policy。

## 4. 时序与冲突

```mermaid
sequenceDiagram
  participant WF as Workflow (old generation)
  participant A as Agents
  participant WF2 as Workflow (new generation)
  participant ST as Process storage
  WF->>WF: 阶段运行中，在 sessions.idle(main) 中等待
  A->>A: Agents 重载（结构性）
  WF->>WF: 卸载：停止运行中的阶段轮次（有界等待）
  WF2->>ST: 加载运行：running 转为 interrupted，活动阶段 lost
  WF2->>WF2: 每个带 report_to 的 interrupted 运行一份报告
```

Workflow 自身重载、依赖（Agents、Collaboration）重载或重启都会把运行结算为 `interrupted`；不重新启动任何内容（E15）。重载是罕见的结构性事件（D3），因此接受这一代价，而不是加以防止。

| 场景 | 序号顺序 | 结果 | 负责方 |
| --- | --- | --- | --- |
| 工具结果之后起始方轮次中断 | result < interrupt | 运行继续 | Workflow |
| 运行很快结束时工具批次尚未结算 | report < `step/tool-result` | Steer 在批次结果之后消费；模型先看到 `run_id` | Loop |
| `workflow.stop` 与阶段自身的 `turn/end` 竞争 | `turn/end` < `stop/interrupt` | 阶段 `completed` 并带结果；运行 `stopped` | 阶段 Session 顺序 |
| 阶段输入在消费前被撤回（用户或 stop） | input < `input/withdrawn` | 阶段 `stopped`；从未消费 | InputQueue、Workflow |
| Workflow、Agents 或 Collaboration 重载，或重启 | — | 运行 `interrupted`，活动阶段 `lost`，不重新启动；各一份报告（E15） | Workflow |
| member 阶段运行中 leader 分支关闭 | close < report | member 阶段继续；报告 `undeliverable` | Workflow |

完整表格：附录 B。

## 5. 上下文映射

| 模型看到的内容 | 形式 | 位置 |
| --- | --- | --- |
| `workflow` 声明 | 已采用配置中的工具声明（仅交互 Session） | 前缀包含该配置的请求 |
| 启动、状态、停止 | `ToolResult.content`（文本，含 `run_id`） | 其批次的工具结果位置 |
| 报告 | 一个 raw 块 `<dim-workflow kind="report" run="..." status="...">`；正文：各阶段状态、最后一步结果的有界预览（A16）、获取完整结果的方式（`status`） | Steer 输入的消费位置；无 key |
| 阶段任务（阶段 Session） | 标记 `<dim-workflow kind="stage" run="..." stage="...">`，随后是作为用户内容的渲染后提示词 | 已启动 Session 的第一个输入，或 member Session 的一个新输入 |

报告和阶段输入均无 key：压缩后只经由摘要保留；子分支只有在切点前已消费时才继承它们。阶段 Session 是非交互的（没有 `ask_user`，没有 `workflow` 工具），因此运行绝不嵌套。模型永远看不到未列出的定义、运行记录、日志、审批、指纹或诊断信息。

## 6. 接口（摘要）

| 接口 | 类型 | 用途 |
| --- | --- | --- |
| `workflow`（`list`、`start`、`status`、`stop`） | `toolContribution`（Session，key `workflow`），风险类别同 Collaboration 的 `agent` 工具 | 模型运行工作流 |
| `workflow.list/save/start/status/stop/approve/revoke` | `managementMethods`（进程，key `workflow`） | 运维方管理定义和运行 |
| `/workflow`、`/workflow:save` | CLI | 调用管理方法 |
| `sessions`、Collaboration `establish` | 依赖 | 创建阶段 Session；input、history、idle、stop、withdraw |
| `dim.workflow/v1` | 此处定义 | 定义格式（附录 A） |

## 7. 安全

项目和插件定义是可执行内容：`start` 和每次阶段启动都需要 Settings 宿主本地批准服务中的授权，记录 `{owner: workflow, node, workspace, source, id: name, fingerprint}`，fingerprint = 规范化定义文档的 SHA-256（R11）。授权只来自在服务 `management` 级别的入口上调用的 `workflow.approve`；工具、Timeline 事实、重放和 fork 永不授权。用户定义无需审批。阶段 policy 遵循 3.3，定义只能收窄它。member 阶段内发起的审批由 Collaboration 转发给 leader（E6），而非 Workflow。阶段 Session 只使用自身资源节点的服务。参数和结果会成为阶段 Session 和报告 Session 的 Timeline 内容；定义不得携带凭据。

## 附录 A. 接口细节

### A.1 定义格式 `dim.workflow/v1`

YAML（A2）。文件：用户来源 `<dim home>/workflows/<name>.yaml`，项目来源 `<workspace>/.dimcode/workflows/<name>.yaml`（E11、A3）；包以 `PluginContents` 类型 `workflows` 发布相同的文档。

```yaml
schema: dim.workflow/v1
name: review-and-fix                 # [a-z0-9-]{1,64}; unique per source
description: Analyze, fix in parallel, summarize
policy: workspace_write              # optional; narrowing only
parameters:
  - { name: target, description: Path to review, required: true }
  - { name: style, default: concise }
steps:
  - stage: { id: analyze, prompt: "Analyze {{params.target}}." }
  - parallel:
      - { id: fix-a, agent: <agent id>, prompt: "Fix part A.\n{{stages.analyze.result}}" }
      - { id: fix-b, runner: member, prompt: "Fix part B.\n{{stages.analyze.result}}" }
  - stage: { id: summarize, model: { provider: p, model: m }, prompt: "Summarize: {{stages.fix-a.result}} {{stages.fix-b.result}}" }
```

| 字段 | 规则 |
| --- | --- |
| `steps` | 按顺序 1–32 步；一步是一个 `stage` 或一个含 2–8 个阶段的 `parallel` 组（A4） |
| `stage.id` | `[a-z0-9-]{1,32}`，在定义内唯一 |
| `stage.agent` | 可选的 Agent id（原生或外部 profile）；默认：Agents 默认 Agent |
| `stage.runner` | `session`（默认）或 `member`；`member` 需要 `report_to` 和 Collaboration `establish` |
| `stage.model` | 可选；否则取 profile 的模型；再否则取 Provider 对 purpose `workflow.stage` 的选择（A5） |
| `stage.prompt` | 渲染后 ≤ 32 KiB 的模板（A6）；占位符 `{{params.<name>}}`、`{{stages.<id>.result}}` |
| `policy` | `read_only` \| `workspace_write` \| `full_access`；受上限约束（3.3） |
| 引用 | `stages.<id>` 必须指向更早一步的阶段；只做纯替换（A7） |
| 参数 | 字符串；缺少必填项或名称未知 → `invalid_parameters`；其余由默认值填充 |

校验发生在外部边界（文件读取、`workflow.save`、`start` 参数）；运行使用已校验的快照。

### A.2 工具 `workflow`

`toolContribution` key `workflow`，仅由 `WorkflowSession` 在配置为 `interactive: true` 的 Session 中注册（A8）。`declaration(): sync -> ToolDeclaration` 返回下列动作；`execute(ToolCall): async -> ToolResult` 调用进程部分；取消即消费方丢弃该调用。风险：Collaboration 的 `agent` 工具所声明的类别（目前没有，因此未分类：ReadOnly 拒绝，WorkspaceWrite 询问，FullAccess 允许；E2）。

| 动作 | 参数 | `ToolResult.content` |
| --- | --- | --- |
| `list` | — | 对调用方 cwd 可见的定义：名称、来源、描述、参数、审批状态 |
| `start` | `name`、`source?`（先 `project`，再 `user`，再 `plugin`）、`params` | `{"run_id", "status": "running", "stages": [...]}`，并附一句说明结果会作为报告输入到达 |
| `status` | `run_id` | 状态、各阶段状态、Session id、有界的结果预览、最近的日志条目 |
| `stop` | `run_id` | 传播后的最终状态 |

`start` 设置 `report_to = {session_id, branch_id}`，从调用方 Session 的配置取 `cwd`/`resource_node`，从起始方取 `policy_cap`（由 Agents 解析）。错误：`definition_not_found`、`invalid_definition`、`invalid_parameters`、`approval_required`、`run_not_found`、`limit_exceeded`（A11）。

### A.3 管理方法

`managementMethods` key `workflow`：`describe(): async -> MethodDecl[]` 列出下列方法（级别 `management`，`mutating` 如表所示）；`call(MethodCall): async -> MethodReply` 以 JSON 参数和结果分派它们。

| 方法 | 参数 | 结果 | `mutating` |
| --- | --- | --- | --- |
| `workflow.list` | `cwd` | 定义及来源、指纹、审批状态、问题 | 否 |
| `workflow.save` | `source: user\|project`、`cwd?`、`document` | `{name, fingerprint}`，通过 Settings 的来源写入（文件权威） | 是 |
| `workflow.start` | `name`、`source?`、`cwd`、`resource_node?`、`params`、`report_to?` | `{run_id}`；`policy_cap` = 宿主配置最大值，默认只读 | 是 |
| `workflow.status` | `run_id?` | 一次运行或最近的运行 | 否 |
| `workflow.stop` | `run_id` | 运行的最终状态 | 是 |
| `workflow.approve`、`workflow.revoke` | `name`、`source`、`cwd` | 绑定到节点 + 工作区 + 指纹的审批状态 | 是 |

已准入的调用在调用方断开后仍会完成。Workflow 重载期间，Rpc 回复 `service_reloading`；未启用时回复 `unknown_method`。

### A.4 运行记录

```text
Run { run_id, name, source: user|project|plugin, fingerprint, definition (snapshot),
      params, cwd, resource_node, policy_cap, report_to?: {session_id, branch_id},
      started_by: tool{session_id, branch_id, call_id} | management,
      state: running | stopping | completed | failed | stopped | interrupted,
      stages: [Stage], log: [LogEntry] (bounded, A9),
      report: none | pending{submission_id} | delivered | undeliverable{reason} }
Stage { id, step, runner, state: waiting | launching | running | completed | failed | stopped | skipped | lost,
        session_id?, agent_id?, submission_id?, input_event_id?, result_text?, reason? }
```

### A.5 依赖的契约

| 契约 | 所有者 | 用途 |
| --- | --- | --- |
| `sessions.launch {agent, cwd, resourceNode?, model?, policy?, starter?}` → `{sessionId}`（非交互；`createdBy = {kind: "launch", source: "workflow"}`） | Agents 补充（[session-access](../contracts/session-access.md)） | 启动的阶段；第一个输入由 Workflow 自己写入 |
| `establish {leaderSessionId, agentId}: async -> {sessionId}` | Collaboration 补充，`sessions` 之外的一个 Collaboration 服务 | member 阶段；Collaboration 维持 Team 规则（单一 leader、`Busy`），应用 leader 的 cwd 和 policy，并把 member 的审批转发给 leader |
| 对自己的输入执行 `sessions.withdraw {sessionId, branchId, inputId}`；按 `submissionId` 执行 `sessions.lookup` | session-access（InputQueue） | 停止时撤回自己未消费的阶段输入 |

### A.6 Timeline 与 Capability 机制

| 步骤 | 路径 |
| --- | --- |
| 启动 | 处理器或管理调用进行校验、检查审批、解析 policy 上限、持久化 `Run{running}`，在进程部分的任务跟踪器上 spawn 运行任务并返回。不写 Timeline |
| 阶段启动 | 在存储中复查（运行为 `running`；审批对快照指纹仍然有效，A12）。持久化 `launching`；`launch` 或 `establish`；持久化 `session_id` 和 `submission_id = workflow:<uuid>` |
| 阶段输入 | `sessions.input {sessionId, branchId: "main", mode: "queue", submissionId, rawBlocks: [marker {kind: "stage", attrs: [run, stage], body: definition name}], content: [rendered prompt]}` → `{inputId}`，存为 `input_event_id`；先执行输入准入检查（`input_denied` → 阶段 `failed: input_denied`）；持久化 `running` |
| 消费 | 阶段 Session 的 InputQueue 和 Loop 在各自的 Dispatcher 上（`input/applied`、`input/consumed`），然后是轮次 |
| 完成 | `sessions.idle {sessionId, branchId: "main"}` 返回截止点 `C`；按页读取 `sessions.history {through: C}`。在 `input/consumed{input_event_id}` 之前：`input/withdrawn` 或其后的 `stop/interrupt` → `stopped`，否则再次等待。消费之后：`(consumed, C]` 中其后的第一个 `turn/end` 决定 `completed`/`failed`/`stopped`；该轮次的最后一条回复文本即结果（Collaboration 的结果规则） |
| 推进 | 持久化阶段结果；某一步的所有阶段都到达终态时，启动下一步或结束运行；失败的阶段会停止运行中的同组阶段并使运行失败（A4） |
| 报告 | 在 `completed`、`failed` 或 `interrupted` 且有 `report_to` 时：持久化 `pending{submission_id}`，在起始分支上以标记 `report` 执行 `sessions.input`（模式 `steer`）；持久化 `delivered`，或在分支缺失、Session 已关闭或 `input_denied` 时持久化 `undeliverable` |
| 停止 | 持久化 `stopping`；等待中的阶段 `skipped`；撤回未消费的阶段输入（`sessions.withdraw`）；对运行中的阶段执行 `sessions.stop {sessionId, branchId: "main"}`；由完成规则读取结果；运行 `stopped`，无报告 |
| 重载、重启 | Session 部分随 Workflow 重载；在途的 `workflow` 调用随其轮次失败。卸载时旧一代停止运行中的阶段轮次（有界等待，A13）。新一代加载运行：`running`/`stopping` → `interrupted`；`launching`/`running` 阶段 → `lost`；不重新启动任何内容。`pending` 报告用 `sessions.lookup` 查询（已接受 → `delivered`，未知 → 现在写入）；带 `report_to` 且 `report: none` 的 interrupted 运行获得一份报告（E15） |

## 附录 B. 完整冲突表

| 场景 | 序号顺序 | 结果 | 负责方 |
| --- | --- | --- | --- |
| 调用分派前起始方中断 | interrupt < `step/tool-call` | 未执行；无运行 | ToolExecutor |
| 处理器运行时起始方中断 | `step/tool-start` < interrupt | 已持久化的运行继续；持久化前取消 → 无运行 | Workflow |
| 工具结果之后起始方中断 | result < interrupt | 运行继续 | Workflow |
| 起始方分支级中断或分支关闭 | close < report | 运行继续；报告 `undeliverable` | Workflow |
| 运行结束时工具批次尚未结算 | report < `step/tool-result` | Steer 在批次结果之后消费 | Loop |
| 报告分支上并发 Steer/Queue | Timeline 顺序 | 按模式应用；报告是普通的 Steer | InputQueue |
| 起始方空闲时报告到达 | — | Steer 启动一个轮次 | InputQueue、Loop |
| 用户撤回报告输入 | report < `input/withdrawn` | 报告计为 `delivered`；不重写 | InputQueue |
| 报告写入与消费之间发生压缩 | input < compaction < consumed | 放在摘要之后 | ContextPreparation |
| 报告写入与消费之间切出子分支 | input < cut < consumed | 子分支看不到它 | InputQueue（子分支） |
| 消费前用户停止阶段 Session | input < interrupt | 阶段 `stopped`；除非撤回，输入在该 Session 中保持待处理 | Workflow |
| 阶段输入被撤回 | input < `input/withdrawn` | 阶段 `stopped` | InputQueue、Workflow |
| `workflow.stop` 与阶段的 `turn/end` 竞争 | `turn/end` < `stop/interrupt` | 阶段 `completed`；运行 `stopped` | 阶段 Session 顺序 |
| `workflow.stop` 与消费竞争 | consumed < withdraw 请求 | 撤回被拒；走 `stop` 路径 | InputQueue、Workflow |
| 两次 `workflow.stop` 或结束后 stop | — | 幂等 | Workflow |
| 重复 `start` | — | 两次运行（A15） | Workflow |
| member 阶段，leader 对同一 member `request` | 阶段输入已排队，尚未开始 | 两个输入都在 member Session 中排队；只有轮次运行后才 `Busy`；阶段结果是其自身输入被消费后的第一个 `turn/end` | Collaboration、Workflow |
| member 阶段进行中 leader 分支关闭 | close < 阶段结束 | member 阶段继续；报告 `undeliverable` | Workflow |
| member 阶段进行中 leader Session 关闭 | — | member 由 Collaboration 关闭 → 阶段 `failed: session_closed`；运行 `failed` | Collaboration、Workflow |
| Agents 或 Collaboration 重载 | — | Workflow 重载；运行 `interrupted`，各一份报告 | Workflow |
| `workflow` 调用期间 Workflow 重载 | — | 调用方 Session 部分重载；该轮次失败（D3，已接受） | Loop |
| 阶段资源节点离线 | — | 因节点不可用而被拒的 launch → 阶段 `failed: node_unavailable`；运行中的阶段 Session 保持加载（不重载，不进入 Pending），其节点服务调用失败为 `node_unavailable`，轮次按自身结果结束（对 [#5](https://github.com/arcships/dim/issues/5) 的要求） | Agents、阶段 Session |
| 轮次中阶段 Session 的 Capability 重载 | 轮次以 `Failed` 结束 | 阶段 `failed` | 阶段 Loop |
| 报告时起始方 Session 已卸载 | — | `sessions.input` 先恢复它，再写入 | Agents |
| 输入准入检查拒绝阶段或报告输入 | — | 不写入任何内容；阶段 `failed: input_denied`（运行 `failed`），或报告 `undeliverable` | Session 输入边界、Workflow |
| 运行中进程重启 | — | `interrupted`/`lost`；已提交但未消费的阶段输入留在各自 Session 中 | Workflow |
| 报告写入结果不确定（重载、发布未知） | — | 重试前查询；绝不产生两份报告 | Workflow |
| 启动后项目定义被编辑 | — | 运行保留其快照；下一次阶段启动复查快照指纹 → `failed: approval_revoked` | Workflow |
| 客户端断开 | — | 无影响 | — |

## 附录 C. 错误与恢复

| 情况 | 结果 |
| --- | --- |
| 定义文件无效 | 连同问题一起列出；`start` → `invalid_definition` |
| 审批缺失或过期 | 启动时 `approval_required`；之后的启动中 `failed: approval_revoked` |
| 声明的 policy 超过上限 | 启动时 `invalid_definition` |
| `launch`/`establish` 失败 | 阶段 `failed`；运行 `failed` |
| 阶段轮次 `Failed`/`Cancelled` | 阶段 `failed`/`stopped`；运行 `failed`（A4） |
| 等待期间 Session 关闭或分支缺失 | 阶段 `failed: session_closed` |
| 存储写入失败 | 运行任务在被守护的副作用之前停止；下次加载时结算为 `interrupted` |

代码即 A.2 中的工具错误代码；管理方法以 `app_error` 返回它们。

## 附录 D. 验收

通过工具、管理方法和 Session 历史读取：

1. 顺序和并行步骤；结果替换；保存时拒绝前向引用。
2. 工具启动返回 `run_id`；报告 Steer 结束一次 `wait`，并在批次结果之后到达。
3. 停止：等待中的阶段 `skipped`，未消费输入被撤回，运行中的阶段被中断，运行 `stopped`，无报告；重复停止幂等。
4. 与阶段 `turn/end` 竞争的停止保留已完成的结果。
5. 起始方轮次中断不停止运行；起始方分支关闭 → `undeliverable`。
6. `.dimcode/workflows/` 中的项目定义：未审批 → `approval_required`；批准；编辑 → 下一阶段 `approval_revoked`。
7. 重启、Workflow 重载和 Agents 重载：`interrupted`/`lost`，不重新启动，恰好一份报告。
8. Policy：WorkspaceWrite 下工具启动 → 阶段最多 WorkspaceWrite；管理启动 → 默认 ReadOnly；ReadOnly 下 `workflow` 被拒。
9. `member` 执行器：member 出现在 leader 的 Team 中；leader 请求与已排队阶段输入竞争时按附录 B 处理。
10. 非交互阶段 Session 没有 `workflow` 工具。

## 附录 E. 假设

| # | 假设 | 依据 | 若被否决的影响 |
| --- | --- | --- | --- |
| A1 | 定义每次调用时读取，不缓存 | 判断（文件权威） | 带变更检测的缓存 |
| A2 | YAML，带 `schema: dim.workflow/v1` | 判断 | 其他语法 |
| A3 | 用户目录 `<dim home>/workflows/` | 判断；overview §B.9 用户来源 | 路径由 Settings 决定 |
| A4 | 阶段或并行组（≤ 32 步，每组 ≤ 8）；快速失败 | 功能清单（“stages”）；限制为判断 | 每阶段的失败策略 |
| A5 | 模型优先级：阶段 > profile > purpose `workflow.stage` | Overview §B.9；优先级为判断 | 其他优先级 |
| A6 | 渲染后提示词 ≤ 32 KiB；存储结果 ≤ 64 KiB | 判断 | 其他上限 |
| A7 | 纯替换模板 | 判断 | 表达式需要沙箱 |
| A8 | 工具仅用于交互 Session | 防止嵌套 | 需要深度限制 |
| A9 | 日志 ≤ 500 条；保留最近 100 次运行 | 判断 | 其他保留策略 |
| A11 | 4 个运行中的运行，每个运行 4 个并发阶段 | 判断 | 其他限制 |
| A12 | 每次阶段启动前复查审批 | MCP 先例 | 只在启动时检查 |
| A13 | 卸载最多等待 10 s 以停止阶段 | 判断 | 其他上限 |
| A15 | `start` 不去重 | 判断 | 幂等键 |
| A16 | 报告预览 ≤ 4 KiB | 判断；`dim-agent-report` 先例 | 其他上限 |

已决定：工具风险和阶段 policy 上限（E2）、`.dimcode/`（E11）、重启后报告一次（E15）、由 Collaboration 转发 member 审批（E6）、`establish`（decisions §2）、进程存储（R8）、撤回（R7）、宿主本地批准服务（R11）、作用域（[ADR 002](../../adr/002-plugin-scope.md)）。

## 附录 F. 待对齐问题

| 问题 | 描述 |
| --- | --- |
| Overview 方法列表 | Overview §B.9 列出 `workflow.list/save/start/status/stop`；本设计增加 `workflow.approve/revoke`（D11） |

## 附录 G. Rust 绑定

中立契约的一种绑定，用于 Rust 实现；TypeScript 实现通过其 rutis-bridge 运行时使用相同的名称和数据。

| 中立 | Rust |
| --- | --- |
| `toolContribution` key `workflow` | 在 `WorkflowSession` 的 `provides` 中注册的 Session 级 `ToolContribution` |
| `managementMethods` key `workflow` | 由 Rpc 收集的 `Contributions<ManagementMethods>` 条目 |
| `sessions.launch` | `AgentIndex::launch(LaunchRequest) -> SessionUse` |
| `sessions.input` / `history` / `idle` / `stop` / `withdraw` | `AgentIndex::acquire(id)` 租约，然后 `Session::input`、`local_page`、`idle`、`stop`、`service::<InputQueue>(branch).withdraw` |
| Collaboration `establish` | `Collaboration::establish(leader_session_id, agent_id) -> SessionUse` |
| 运行任务 | 在 `ctx.effect` 持有的 `TaskTracker` 上 spawn |
