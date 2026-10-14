# ExternalAgent

状态：草案。总体设计：[overview §B.10](../overview.md)；作用域：[ADR 002](../../adr/002-plugin-scope.md)（进程）。阶段：P3。语言：优先 TypeScript（[overview §F.3](../overview.md)）。

ExternalAgent 让本地安装的编码 agent（Claude Code、Codex、Cursor、OpenCode、ZCode 等）可作为一种模型连接使用。模型为外部 agent 的 Session 照常经由 Loop 和 ModelProvider 运行：ExternalAgent 为类型 `external-agent` 提供 `modelKind` 服务，由 Aimux 的 ModelHost 路由到该服务（Aimux 补充），它把每个 dim 模型请求转换为一次外部轮次，再把外部流转换回模型输出块。外部 agent 使用自己的工具；它们的权限请求在流中以审批询问的形式返回，由 ModelProvider 在执行该调用的 Session 中作为普通 dim 审批准入。ExternalAgent 不写自己的 Timeline 事件。所有契约都是语言中立的命名服务（[ADR 002 D5](../../adr/002-plugin-scope.md)）；Rust 形式见[附录 G](#附录-g-rust-绑定)。

## 1. 结构

```mermaid
flowchart LR
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
  subgraph proc["进程，控制节点"]
    EA["ExternalAgent"]:::new
    MM["managementMethods key external_agents"]:::new
    Aimux["Aimux ModelHost（按类型路由 modelKind，补充）"]
    Rpc["Rpc 方法表"]
    Store["进程存储：记录、external_agents 配置"]
    Agents["Agents：profiles、sessions"]
    Settings["Settings：凭据解析器、类型化加载器操作"]
    Provider["Provider：连接条目"]
  end
  subgraph node["进程，每个有已启用 agent 的节点"]
    Bridge["ExternalAgentBridge 行：externalAgentNode key node"]:::new
    HP["HostPlatform（启动、隔离）"]
  end
  subgraph branch["调用方 Session 的分支"]
    MP["ModelProvider（调用作用域）"]
    Appr["Approval"]
  end
  EA --> MM
  Rpc -->|"按名称收集 managementMethods"| MM
  MP -->|"带调用作用域 invoke"| Aimux
  Aimux -->|"modelKind external-agent：invoke、answerApproval"| EA
  MP -->|"准入流中的审批询问"| Appr
  EA -->|"sessions.history"| Agents
  EA --> Store
  EA --> Settings
  EA -->|"连接条目"| Provider
  EA -->|"profiles"| Agents
  EA -->|"按节点收集 externalAgentNode"| Bridge
  Bridge --> HP
```

外部 agent 连接、记录和每节点进程在多个 Session 之间共享，因此 ExternalAgent 是进程插件（[ADR 002](../../adr/002-plugin-scope.md) D1）。它依赖同一作用域的 Aimux、Agents、Provider 和 Settings。按类型路由是 Aimux 补充：ModelHost 按名称收集 `modelKind` 服务，并把解析出的连接类型为 `external-agent` 的请求路由到具有该 key 的实例。依赖遵循作用域（D3）：Aimux、Agents、Provider 或 Settings 重载会重载 ExternalAgent，并使其在途外部调用失败；这一罕见的结构性代价已被接受并有文档记录。ModelProvider 不依赖 ExternalAgent：在没有注册 key 为 `external-agent` 的 `modelKind` 实例期间，该类型的请求在接受前失败（暂时性），其他类型不受影响。每个 `ExternalAgentBridge` 是绑定到一个节点的结构性加载器行，仅在该节点上至少有一个 agent 启用时存在；它依赖其节点的 HostPlatform，并在其 `provides` 中声明 key 为 `<node>` 的 `externalAgentNode`，由 ExternalAgent 按名称收集。启用状态变化时，ExternalAgent 通过 Settings 的加载器行操作添加和移除桥接行（D4）。对等桥接经 rutis 链路导入；链路断开期间导入保持注册，其调用失败为 `node_unavailable`，因此不会重载任何内容，只有指向该节点的调用失败（对 [#5](https://github.com/arcships/dim/issues/5) 的要求）。ExternalAgent 从不访问 Approval：由 ModelProvider 准入流中携带的审批询问。

| 部分 | 作用域 | 依赖 | 提供 |
| --- | --- | --- | --- |
| `ExternalAgent` | 进程，控制节点 | Aimux（ModelHost 按类型路由）、Agents（profile 注册、用于读取历史的 `sessions`）、Provider 连接条目操作、Settings（凭据解析器、用于桥接行的加载器行操作）、App Server 进程存储（记录和 `external_agents.*` 业务配置） | `modelKind` key `external-agent`；`managementMethods` key `external_agents`；收集 `externalAgentNode` |
| `ExternalAgentBridge` | 进程，每个有已启用 agent 的节点一个结构性行：本地行，其他节点为 `peer:<node>/ExternalAgentBridge`（[overview §6.3](../overview.md)） | 该节点的 `HostPlatform` | `externalAgentNode` key `<node>`，经链路导出 |

**依赖的契约**：Aimux 补充（ModelHost 按名称收集 `modelKind`，并带调用作用域按类型路由，A.2）；另外两个补充：**ModelProvider** 通过其分支的 Approval 准入 `approval_ask` 流条目，并用 `answerApproval` 回答，**Approval** 在模型步骤打开期间接受 `Host` 来源的准入，复用 `approval/request`、`approval/respond` 和 `approval/cancelled`（无新事件）；现有模型契约的模型块和失败数据（[host.rs](../../../../crates/dim-agent-capabilities/src/host.rs)）以及 ModelProvider 的失败策略（[recovery.rs](../../../../crates/dim-agent-capabilities/src/model_provider/recovery.rs)）；Approval 的 `admit`（[Approval](../../../architecture/rust-agent/plugins/Approval.md)）；Collaboration 补充“member 审批转发给 leader”（decisions E6）；`sessions`（[session-access](../contracts/session-access.md)）；由 Rpc 收集的 `managementMethods`（[session-boundary](../contracts/session-boundary.md)）；App Server 进程存储和 Settings（现有的类型化服务，待 [#4](https://github.com/arcships/dim/issues/4) 暴露后可由 TypeScript 部分按名称访问）。**此处定义的契约**：`modelKind` key `external-agent` 及其目标 `{node, agent}`；`externalAgentNode`；对话记录；审批调用形态 `external_agent.<kind>`；业务配置 `external_agents.*`；方法 `external_agents.*`；与 [AcpGateway](AcpGateway.md) 共享的 ACP schema 和一致性测试套件（客户端侧）。Provider、Agents、Approval 和 Collaboration 从不指名本插件。

## 2. 数据与状态

```mermaid
flowchart TB
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
  classDef contract stroke-dasharray:4 3
  Cfg["业务配置 external_agents.node.agent.*（进程存储）"]:::new
  Row["加载器行 peer:node/ExternalAgentBridge（结构性）"]
  Conn["Provider 连接条目 external-agent:node:agent"]
  Prof["Agents profile：Agent (node)，无工具"]
  Rec["每个 Session + 分支一个 ConversationRecord（进程存储）"]:::new
  Cat["每节点目录（桥接内存）"]:::new
  Live["实时 ACP 连接和 CLI 进程（桥接）"]:::new
  TL["分支 Timeline：model/call、step/*、input/*、approval/*"]:::contract
  Cfg -->|"在 apply 和 set_enabled 时对齐"| Conn
  Cfg -->|"对齐"| Prof
  Cfg -->|"节点上有 agent 启用时行存在"| Row
  TL -->|"通过 sessions.history 读取的页"| Rec
  Rec -->|"选择 Continue 或 New"| Live
  Cat -->|"external_agents.list"| Cfg
```

`external_agents.*` 条目是业务配置（[ADR 002](../../adr/002-plugin-scope.md) D4），保存在 ExternalAgent 的进程存储中，在使用时读取；变更是数据更新加通知，绝不重载。它们是启用状态的事实来源；连接条目、profile 和桥接行通过各自所有者的类型化操作从它们对齐而来，因此可重建。只有桥接行是结构性的，增删一个桥接行只会重载该桥接。对话记录只保存投递游标和外部身份；每次投递决策都根据 Timeline 页加记录重新计算。桥接不持有任何必须存续的内容。

```text
ConversationRecord {
  key: (session_id, branch_id),
  target: { node, agent },
  external_id: String,          // ACP sessionId or CLI conversation id
  cursor: u64,                  // sequence of the model/call of the last accepted call on this branch
  scan_from: u64,               // lowest local sequence that may hold an input/message not yet consumed
  last_request_id: String,
  state: ready | in_flight { request_id } | uncertain,
}
```

记录存放在 App Server 进程存储中本插件的根目录下（原子替换、格式版本）。在 `apply` 时，每条 `in_flight` 记录变为 `uncertain`；记录在最后使用后保留 30 天（A12）。

## 3. 流程

### 3.1 模型调用（初始化或继续）

```mermaid
sequenceDiagram
  participant TL as Branch Timeline
  participant MP as ModelProvider
  participant MH as Aimux ModelHost
  participant EA as ExternalAgent
  participant SA as Calling Session (via sessions)
  participant BR as Bridge (agent node)
  TL->>MP: model/call（Dispatcher 处理器注册调用任务后返回）
  MP->>MH: invoke(request, connection, call scope, cancel)
  MH->>EA: modelKind[external-agent].invoke
  EA->>SA: sessions.history(branch, after scan_from, through call_sequence)
  EA->>EA: 决定 New 或 Continue（仅依据记录 + 页）
  EA->>EA: 持久化 in_flight（进程存储）
  EA->>BR: externalAgentNode[node].turn(TurnSpec)
  BR-->>EA: Accepted、Text、Thought、ToolActivity、End
  EA->>EA: 持久化 cursor、external_id、ready
  EA-->>MP: 模型块流
  MP->>TL: step/text 和 step/thinking（append），然后 step/model-output-end（append_batch_registered）
```

决策只读取 `call_sequence` 及之前的事实（已分派，因此已发布，且是批次边界）和记录；它从不读取其他插件的内存。记录在外部轮次之前写入，因此重启后可以知道某个轮次可能已经开始。

投递决策，给定记录 `R`、请求 `Q`、作用域 `S`：

1. 扫描 `S.branch` 在 `(R.scan_from, S.call_sequence]` 中的本地事件（无可用记录时扫描整个本地分支）：`input/message`、序号 `> R.cursor` 的 `input/consumed`、序号 `> R.cursor` 的输出事件（`step/text`、`step/thinking`、`step/tool-use-start`）。
2. **初始化**：没有记录、`R.target` 不同、`R.state = uncertain`，或范围内的某个输出属于 `R.last_request_id` 以外的请求（另一个模型作答过）。打开 `New`，发送 `Q.messages` 的转录（§5）以及回答最后一条用户消息的指令。
3. **继续**：其他情况。`Continue { external_id }`，按消费顺序附带范围内已消费的输入。
4. 新的 `scan_from` = 收集到的、在 `S.call_sequence` 时尚未消费的 `input/message` 的最小序号，否则为 `S.call_sequence`。增量为空时，在接受前以 `external_agent_no_input` 失败（A8）。

子分支上的 `Q.messages` 是直到切点的祖先前缀加上本地事件，因此子分支根据自身历史初始化，从不触及父分支的对话；fork（新的 Session id）同样如此。

### 3.2 外部权限请求

```mermaid
sequenceDiagram
  participant BR as Bridge (ACP client)
  participant EA as ExternalAgent
  participant MP as ModelProvider (calling branch)
  participant AP as Approval (calling branch)
  participant TL as Branch Timeline
  participant U as Leader model or user
  BR->>EA: permission_asked(ask_id, title, kind, options)
  EA-->>MP: 流：思考行，然后 approval_ask（经 ModelHost）
  MP->>AP: admit（Host 来源），步骤仍打开
  AP->>TL: 仅 Ask：approval/request（append_registered）
  U->>AP: approval.respond（leader 经 Collaboration E6，否则为用户）
  TL->>AP: approval/respond（Dispatcher，按序号），复查 policy
  AP-->>MP: Ok、denied 或 cancelled
  MP->>EA: answerApproval(request_id, ask_id, outcome)
  EA->>BR: 回答：allow_once、reject_once 或 cancelled
```

权限请求是执行外部调用的 Session（对于委派的 member，即 member Session）中的一次普通调用准入，由 ModelProvider 通过 §1 的两个补充完成：Permission 的预设先作决定，只有 `Ask` 才写入 `approval/request`。转发给 leader 是 Collaboration 的规则（E6）；ExternalAgent 只看到结果。询问之前先发出一行思考，因此调用在等待时已有可见输出（§4）。

映射：ACP `toolCall.kind` → 调用名 `external_agent.<kind>` 和风险：`read`、`search` → `read_filesystem`；`edit`、`delete`、`move` → `write_filesystem`；`execute` → `execute_process`；`fetch` → `network`；其他 → 未分类（A7）。`admit` 为 `Ok` 时选择一个 `allow_once` 选项；拒绝时选择 `reject_once`；所需的单次选项不存在或询问被取消时，桥接以结果 `cancelled` 回答。绝不选择 `*_always`。非交互 CLI 适配器没有请求通道：其启动参数让 agent 处于自身的非自动批准模式，因此 agent 无法询问的请求由 agent 自行拒绝（A16）。

### 3.3 委派与中断

1. 资源节点为 `build-box` 的 leader Session 通过 `agent` 工具请求“Codex (build-box)”。Collaboration 用该 profile 创建一个 member Session；member 继承 leader 的资源节点 `build-box`，因此桥接在 `build-box` 上于 member 的 cwd 中运行 Codex。位于其他节点的 leader 在接受前得到 `external_agent_wrong_node`（A13）。
2. 中断：ModelProvider 取消调用令牌；桥接发送 ACP `session/cancel`（或向 CLI 发信号），以 `cancelled` 回答所有待处理的权限请求，排空直至 prompt 响应，并在 10 s 宽限期（A4）后终止进程。已收到的文本保留；若取消已被确认，下一个输入继续同一对话。

## 4. 时序与冲突

```mermaid
sequenceDiagram
  participant TL as Branch Timeline
  participant MP as ModelProvider
  participant AP as Approval
  participant EA as ExternalAgent
  participant BR as Bridge
  Note over TL: approval/request 位于序号 n，调用等待中
  alt approval/respond 位于 n+1，先于 n+2 的 stop/interrupt
    TL->>AP: respond 先被消费：复查后 admit 返回 Ok
    EA->>BR: allow_once
    TL->>MP: stop/interrupt：取消调用
    EA->>BR: session/cancel、排空、Cancelled
  else stop/interrupt 位于 n+1，先于任何 respond
    TL->>MP: interrupt 被消费：调用令牌取消
    AP->>TL: approval/cancelled（Approval，请求的发起方）
    EA->>BR: 回答 cancelled，然后 session/cancel
    Note over AP: 之后的 approval.respond 返回 Unknown
  end
```

回答与中断的先后由调用方分支上序号的 Dispatcher 顺序决定；请求总是由 Approval 结束。外部 agent 只得知胜出的决定。

| 场景 | 顺序 | 结果 | 负责方 |
| --- | --- | --- | --- |
| 审批待处理，长时间无回答 | — | dim 无超时；调用保持打开；用户或 leader 可中断（A17） | Approval、Collaboration |
| dim 询问期间外部 agent 撤回其请求或结束其 prompt | 桥接 `permission_withdrawn` | ExternalAgent 以 `approval_ask_withdrawn` 流条目结束询问；ModelProvider 取消准入；Approval 写入 `approval/cancelled`；迟到的回答返回 Unknown | ExternalAgent、Approval |
| leader 不可用（已卸载、轮次已结束、其对 member 的请求已停止） | 按 Collaboration | 由 Collaboration 补充决定投递或回退给用户；停止 member 请求会中断 member 轮次（上一行） | Collaboration |
| 审批待处理时 Steer | 思考行之后的 `input/applied` | 不替换（已有可见输出）；Loop 在下一个请求中投递 Steer | Loop |
| 有待处理审批时 ExternalAgent 重载 | — | 调用得到“provider unavailable”（而非 `capability reloading`）；询问取消；轮次流被丢弃，因此桥接取消并终止进程；记录 `uncertain`；桥接不重载 | ExternalAgent、ModelProvider |
| agent 节点离线 | 链路断开；桥接导入保持注册 | 该节点的调用在接受前失败为 `node_unavailable`（暂时性）；不重载任何内容；其他节点不受影响（对 #5 的要求） | ExternalAgent |
| 调用中进程重启 | `model/call` 和 `approval/request` 未结束 | Approval 恢复写入 `approval/cancelled`；ModelProvider 不调用即令该调用失败；记录 `uncertain`；不重发任何内容 | Approval、ModelProvider |

其余所有行：[附录 B](#附录-b-完整冲突表)。

## 5. 上下文映射

| 受众 | 看到的内容 | 形式 |
| --- | --- | --- |
| 外部 agent，初始化 | `Q.messages` 作为一个文本 prompt `<dim-history>`：`[user]`、`[assistant]`、`[tool call <name>] <input JSON>`、`[tool result] <content text>`，图片为 `[image]`，省略思考（A14）；然后是指令 | ACP 文本块或 CLI stdin |
| 外部 agent，继续 | 每个已消费输入：原样的 raw 块文本，然后是用户内容（agent 声明 `promptCapabilities.image` 时图片为 ACP 图片块，否则为 `[image omitted]`） | Prompt 块 |
| 外部 agent，永不 | dim 系统提示词、提示词来源、工具声明、采样参数、`metadata`、其他插件的事件、诊断信息；`config.reasoning` 只经由适配器的推理标志 | — |
| 调用方 Session 的历史 | 外部回复为 `step/text`；工具活动和权限行为 `step/thinking`（`[tool] <title>: <status>`、`[permission] <title>: <state>`） | 模型输出事实 |
| leader 模型（member 审批） | 审批的 `call`（`external_agent.<kind>`、`{agent, title, kind, raw_input}`，上限 4 KiB，A18），由 Collaboration 在 `agent` 工具中渲染 | Collaboration 的工具结果 |
| 用户 | 带相同调用形态的 `approval.request` | 客户端 UI |

带 key 的块在消费时投递一次；“生效中”和退役在外部没有意义，重新初始化只发送投影所保留的内容。已提交的外部输出与任何模型输出一样进入历史：压缩会对其做摘要，子分支继承直到切点的部分，之后的原生模型将其视为 assistant 消息，被中断的输出保留文本并丢弃思考。Collaboration 和 Workflow 的结果读取最后一个请求的文本，因此工具活动绝不污染结果。非交互 Session 的行为完全相同；审批映射也相同，因为审批不是 `ask_user`。

## 6. 接口（摘要）

| 接口 | 类型 | 用途 |
| --- | --- | --- |
| `external_agents.list` / `set_enabled` / `configure` | `managementMethods`（进程，key `external_agents`），级别 `management` | 每节点检测、启用、配置（[A.1](#a1-方法级别-management)） |
| `modelKind` key `external-agent`，目标 `{node, agent}` | 由 Aimux 的 ModelHost 路由的进程服务（Aimux 补充） | 外部 profile 的模型调用（[A.2](#a2-provider-注册与调用作用域)） |
| `externalAgentNode` key `<node>` | 由每个桥接行提供，由 ExternalAgent 收集 | `detect`、带可回答权限询问的 `turn`（[A.3](#a3-桥接服务与适配器)） |
| Approval 调用 `external_agent.<kind>` | ModelProvider 发起的 Approval 准入（Host 来源） | 外部权限请求 |
| ACP schema 和一致性测试套件 | 与 AcpGateway 共享 | 客户端侧（[A.5](#a5-共享-acp-schema)） |

## 7. 安全

外部 agent 以自己的账户和工具运行；dim 只在它们询问时，通过调用方 Session 的 policy 和 Approval 准入其操作，无法阻止 agent 不经询问而采取的操作：节点 HostPlatform 的隔离是强制边界，重放或 fork 绝不扩大它。绝不选择 `*_always`。凭据是启动时由 Settings 解析器从引用解析出的环境变量值，绝不写入记录、Timeline、诊断信息或回复。启用 agent 是宿主配置，只能通过 `management` 级方法进行；模型无法启用 agent。可执行文件在节点上解析，绝不来自项目文件。

## 附录 A. 接口细节

### A.1 方法（级别 `management`）

`managementMethods` key `external_agents`：`describe(): async -> MethodDecl[]` 列出下列方法；`call(MethodCall): async -> MethodReply` 以 JSON 参数和结果分派它们。

| 方法 | 参数 | 结果 | `mutating` |
| --- | --- | --- | --- |
| `external_agents.list` | `refresh?` | 每个节点：agent、可执行文件、版本、协议（ACP 或 CLI）、启用状态、配置（仅引用）；离线节点标记为 `node_unavailable` | 否 |
| `external_agents.set_enabled` | `node`、`agent`、`enabled` | 新状态 | 是 |
| `external_agents.configure` | `node`、`agent`、`executable?`、`args?`、`env?: {name: credential reference}`、`reasoning?` | 新配置 | 是 |

检测在加载时和 `refresh` 时调用 `externalAgentNode.detect`（A1）；检测到的 agent 在启用前保持禁用（A3）。`set_enabled` 和 `configure` 更新进程存储中的业务配置并通知；它们绝不重载 ExternalAgent。在某节点上启用第一个 agent 会添加该节点的桥接行，禁用最后一个则移除它。启用 `(node, agent)` 会通过 Provider 的类型化连接条目操作对齐条目 `external-agent:<node>:<agent>`（含一个模型 `default`（A2）和目标 `{node, agent}`），并通过 Agents 的 profile 注册对齐一个 profile `{ name: "<Agent> (<node>)", system_prompt: "", tools: [], model: { provider_id: "external-agent:<node>:<agent>", model_id: "default" } }`。禁用会移除两者；现有 Session 保留其已采用的模型，并在下一次调用时以 `external_agent_disabled` 失败。Profile 与任何 Agent 一样由 Agents 列出，因此 Collaboration 的 `agent` 工具可以请求它们。

### A.2 Provider 注册与调用作用域

`modelKind`，进程作用域，key = 连接类型（`external-agent`）。ModelHost（Aimux 补充）按名称收集实例，并按解析出的连接类型路由。

| 方法 | 模式 | 请求 | 回复 |
| --- | --- | --- | --- |
| `invoke` | async，流式 | `{request: ModelRequest data, target: {node, agent}, scope: {sessionId, branchId, turnId, requestId, callSequence, cwd, resourceNode, interactive, collaborationRole}}` | 条目流：模型块（A.4）、`{approval_ask: {askId, operationId, call: {name, input}, risk?}}`、`{approval_ask_withdrawn: {askId}}`；以停止块或错误 `{code, origin, transient}` 结束 |
| `answerApproval` | async | `{requestId, askId, outcome: allow \| deny \| cancelled}` | `{}`；未知询问 → `{}`（迟到的回答被忽略） |

取消是一个协议动作：消费方丢弃 `invoke` 流或发送其取消请求；随后内部以 `cancelled` 回答打开的询问。`operationId = "external-agent:<requestId>:<ask_n>"`。ModelProvider 补充：收到 `approval_ask` 时，以来源 `Host {operationId}` 和该调用调用其分支的 `Approval::admit`，并通过 ModelHost 的 `answerApproval` 返回结果；收到 `approval_ask_withdrawn` 时取消该准入。Approval 补充：此类准入在 `step/model-start` 打开期间被接受；其事实是普通的审批事实，对模型无可见影响。当请求所采用的 Provider 连接条目 `external-agent:<node>:<agent>` 存在、但没有注册该 key 的 `modelKind` 实例时，ModelHost 在接受前令其失败（暂时性）；其他类型不受影响。

### A.3 桥接服务与适配器

`externalAgentNode`，进程作用域，key = 节点；由该节点的桥接行提供，由 ExternalAgent 收集。

| 方法 | 模式 | 请求 | 回复 |
| --- | --- | --- | --- |
| `detect` | async | `{}` | `[{agent, executable, version, protocol: acp \| cli}]` |
| `turn` | async，流式 | `TurnSpec {turn_id, agent, config, cwd, conversation: {new} \| {continue: {external_id}}, prompt: PromptBlock[]}` | `TurnUpdate` 流：`{accepted: {external_id}}`、`{text}`、`{thought}`、`{tool_activity: {id, title, status}}`、`{permission_asked: {ask_id, title, kind, raw_input?, options}}`、`{permission_withdrawn: {ask_id}}`、`{usage}`、`{end: reason}`；或错误 `{code, transient}` |
| `answer` | async | `{turn_id, ask_id, answer: allow \| reject \| cancelled}` | `{}` |

取消：消费方丢弃 `turn` 流或发送其取消请求；随后桥接执行下表中适配器的取消路径。节点链路断开期间，每个调用都失败为 `node_unavailable`。

| 适配器 | 打开 | 轮次 | 取消 |
| --- | --- | --- | --- |
| ACP 客户端 | 通过节点的 HostPlatform（cwd、隔离）spawn ACP 命令；以关闭 fs 和 terminal 客户端 Capability 的方式 `initialize`；`session/new {cwd, mcpServers: []}`，或在声明了 `loadSession` 时对已知 `external_id` 执行 `session/load` | `session/prompt`；`session/update` 流；`session/request_permission` → `permission_asked`；请求写出后即 `accepted` | `session/cancel`，以 `cancelled` 回答待处理的权限请求，排空；宽限期后终止（A4） |
| 非交互 CLI | 每个轮次 spawn，继续时带对话 id；限制性权限标志（A16） | 在 stdin 上传入 prompt，按适配器解析结构化输出；spawn 且 stdin 完整写入后即 `Accepted` | 中断信号、宽限期、终止 |

每个对话一个 ACP 进程，空闲 10 分钟后关闭（A5）。适配器定义是本插件内的数据（A6）。

### A.4 流映射与请求字段

| 外部 | `ModelChunk` |
| --- | --- |
| `Accepted` | `MessageStart { message_id: "<external_id>:<request_id>" }` |
| 消息块 / CLI 文本 | `Text(delta)`；先关闭打开的思考块 |
| 思考块 / CLI 推理 | `Thinking(delta)`；先以 `TextEnd` 关闭打开的文本块 |
| `tool_call`、`tool_call_update` 状态变化 | `Thinking("[tool] <title>: <status>")`（A9）；绝不为 `ToolStart`（未提供工具；ModelProvider 拒绝未提供的调用） |
| 权限已询问 / 已决定 | `Thinking("[permission] <title>: asked")`，然后是 `allowed`、`denied` 或 `cancelled` |
| `plan`、`available_commands_update`、`current_mode_update`、由 `session/load` 重放的历史 | 不转发；进入诊断信息 |
| 适配器报告的用量 | `Usage`（A10） |
| 结束 `end_turn` | 关闭打开的块，`MessageStop { reason: EndTurn }` |
| 结束 `max_tokens` | `MessageStop { reason: MaxTokens }` |
| 结束 `max_turn_requests`、`refusal`、CLI 非零退出 | 失败，非暂时性（A11） |
| 我方取消后结束 `cancelled` | 流结束且无 `MessageStop`；ModelProvider 报告 `Cancelled` |

不发出 `ModelChunk::Content`。请求字段：`messages` 只在初始化时使用；`system`、`tools`、采样字段、`provider_options`、`metadata`、`capability_snapshot` 不转发。

重试安全：`Accepted` 之前可重试的失败（spawn 失败、连接被拒、`node_unavailable`）是带 `{origin: transport, transient: true}` 的失败；其他所有失败都是非暂时性的。ModelProvider 在非暂时性错误时停止，且一旦收到非 `MessageStart` 块即停止，因此 agent 可能已开始的轮次绝不会被发送两次。`Accepted` 之前记录不变，因此重试计算出相同的增量。

### A.5 共享 ACP schema

固定的协议版本、按语言生成的类型、与 AcpGateway 共享的基于转录的一致性测试套件；fixtures 覆盖 initialize、new/load、带文本和图片的 prompt、流式更新、prompt 期间的取消、分别以 allow、reject 和 cancelled 回答的权限请求、取消期间的权限请求，以及 prompt 中途进程退出。不共享任何运行时代码（[overview §B.15](../overview.md)）。

### A.6 各步骤机制

| 步骤 | 路径 |
| --- | --- |
| 请求 | Loop 写入 `input/consumed`* 和 `step/model-start`；ContextPreparation 写入 `model/call`（`Timeline::append`）；ModelProvider 在其 Dispatcher 上消费它，注册调用任务后返回 |
| 解析 | 在任务中：Provider 解析 `{kind: external-agent, target}`；ModelHost 带调用作用域调用 `modelKind[external-agent].invoke` |
| 读取 | `sessions.history {sessionId, branchId, after: scan_from, through: call_sequence}`；无法读取的 Session 在接受前失败，暂时性 |
| 预写 | 发送前持久化 `in_flight`；收到 `Accepted` 时持久化 cursor、`last_request_id`、`scan_from`、`external_id`；在 `Accepted` 之前结束的尝试恢复记录 |
| 输出 | ModelProvider 提交块（`Timeline::append`），并用 `append_batch_registered` 写入 `step/model-output-end`；`StateCell<ModelProgress>` 显示未提交的文本 |
| 审批 | ModelProvider 准入 `approval_ask`；Approval 以 `append_registered` 写入 `approval/request`（等待方在提交锁内注册）；`approval/respond` 在分支 Dispatcher 上消费；其他所有结束情况下 Approval 写入 `approval/cancelled`；ModelProvider 调用 `answerApproval` |
| 结束 | 外部结束或已确认的取消 → `ready`；`Accepted` 之后终止或链路丢失 → `uncertain` |
| 重载 | 卸载时 ExternalAgent 以 `cancelled` 结束其打开的询问，丢弃其 `turn` 流（桥接取消并终止进程），其 `modelKind` 实例离开；每个调用以 provider-unavailable 失败结束，绝不是 `CAPABILITY_RELOADING`；桥接不重载 |
| 重启 | ModelProvider 重建使未结束的 `model/call` 失败；Approval 重建取消未匹配的请求；`apply` 把 `in_flight` 记录标记为 `uncertain` |

ExternalAgent 本身不追加任何内容，不订阅任何 Dispatcher，也不拥有 Session 状态。

## 附录 B. 完整冲突表

| 场景 | 顺序 | 结果 | 负责方 |
| --- | --- | --- | --- |
| `model/call` 被消费前轮次中断 | interrupt < `model/call` | 不调用即 `Cancelled` | ModelProvider |
| `Accepted` 之前中断 | — | 尝试取消；未发送任何内容；记录不变 | Bridge、ModelProvider |
| `Accepted` 之后中断 | `model/call` < interrupt | 发送取消；直至 prompt 响应的更新被提交；`Cancelled`；记录 `ready`，cursor 前移 | Bridge、ModelProvider |
| 宽限期内取消未被确认 | — | 终止；`Cancelled`；`uncertain`；下一次调用初始化 | Bridge |
| `approval/respond` 先于中断 | respond < interrupt | 投递决定；然后如上取消 | Approval |
| 中断先于 `approval/respond` | interrupt < respond | `approval/cancelled`；询问以 `cancelled` 回答；迟到的 respond 返回 Unknown | Approval |
| 询问期间 policy 变化 | `permission/changed` < respond | allow 后复查；`Deny` → `approval/cancelled`，询问以 reject 回答 | Approval |
| 预设不经询问即作决定 | — | 立即返回 `Allow`/`Deny`；无事件 | Permission |
| 一次调用中有多个询问 | — | 按到达顺序进行独立审批 | Approval |
| 取消排空期间到达的权限请求 | — | 不询问即以 `cancelled` 回答 | Bridge |
| 外部 agent 撤回其请求 | `permission_withdrawn` | 询问取消；`approval/cancelled` | ExternalAgent、Approval |
| member 审批时 leader 不可用 | — | 由 Collaboration 补充决定；ExternalAgent 等待事实 | Collaboration |
| 无回答、无中断 | — | 无限等待（A17） | — |
| 任何可见块之前 Steer | 调用期间 `input/applied`（Steer） | ModelProvider 替换请求并取消；下一个请求投递该 Steer（以及未被接受的更早输入） | ModelProvider、Bridge |
| 可见输出（含权限行）之后 Steer | — | 不替换；同一轮次中的新请求 | Loop |
| 已排队输入的 `input/withdrawn` | 消费前撤回 | 从未消费，因此从未投递；消费推进越过它后 `scan_from` 跳过它 | InputQueue、Bridge |
| 并发 Queue/Append 输入 | Timeline 顺序 | 按消费顺序投递 | Loop、Bridge |
| 工具批次未结算 | — | 这些请求中没有 dim 工具调用 | — |
| 调用前压缩 | `context/compaction` < `model/call` | 继续不受影响；初始化包含摘要 | ContextPreparation、Bridge |
| 切出子分支，包括在 `approval/request` 与 respond 之间 | 任意 | 子分支没有记录，进行初始化；Approval 只跟踪本地请求，因此父分支的请求仍属于父分支 | Bridge、Approval |
| 调用中分支中断或关闭 | 分支中断 | 同中断；Approval 关闭时取消询问 | ModelProvider、Approval |
| 中间夹有原生模型 | cursor 之后的外来输出 | 重新初始化 | Bridge |
| 接受前暂时性失败 | — | 以相同增量重试 | ModelProvider |
| 调用中 ExternalAgent 重载 | — | provider-unavailable 失败；终止；`uncertain`；ModelProvider 和桥接不重载 | ExternalAgent |
| Agents、Aimux、Provider 或 Settings 重载 | — | ExternalAgent 重载（D3，已接受）；在途外部调用如其自身重载时一样失败 | ExternalAgent |
| 调用中进程重启 | 未结束的事实 | 由重建结算；`uncertain`；不重发任何内容 | ModelProvider、Approval |
| Session 卸载与恢复 | — | 记录保留；ACP 进程因空闲关闭；以 `session/load` 重新打开或初始化 | Bridge |
| agent 节点离线 | 链路断开；导入保持注册 | 接受前 `node_unavailable` 暂时性失败，在上限内重试，然后 `Failed`；不重载任何内容；其他节点不受影响（对 #5 的要求） | ExternalAgent |
| `Accepted` 之后链路丢失 | — | 非暂时性失败；询问取消；`uncertain` | Bridge |
| Session 资源节点与 agent 节点不同 | — | `external_agent_wrong_node`，非暂时性（A13） | ExternalAgent |
| 重复 `set_enabled` | — | 幂等 | ExternalAgent |
| 客户端断开 | — | 无影响；待处理审批等待另一个客户端或 leader | — |

## 附录 C. 错误与恢复

| 代码 | 是否在接受前 | 含义 |
| --- | --- | --- |
| `external_agent_not_installed` | 是，非暂时性 | 节点上未找到可执行文件 |
| `external_agent_auth_required` | 是，非暂时性 | agent 需要自身登录；展示中给出命令 |
| `external_agent_launch_failed` | 是，暂时性 | spawn 或连接失败 |
| `node_unavailable` | 是，暂时性 | agent 已在节点上启用，但节点链路断开；桥接导入令调用失败 |
| `external_agent_disabled`、`external_agent_wrong_node`、`external_agent_no_input` | 是，非暂时性 | A.1、§3.3、§3.1 |
| `external_agent_protocol` | 两者皆可 | 输出格式错误；终止；若在接受后则 `uncertain` |
| `external_agent_exited` | 否 | 轮次中进程退出或链路丢失；`uncertain` |
| provider unavailable | 两者皆可 | ExternalAgent 正在卸载；先取消询问 |

错误以失败的模型输出呈现；已提交的文本保留。继续失败或 `uncertain` 记录会触发重新初始化（A15）。

## 附录 D. 验收

使用脚本化的 ACP agent 和脚本化的 CLI，通过公共接口进行流程测试：

1. 第一次调用初始化；第二次只投递新输入；Append 输入随消费它的请求投递；被撤回的 Queue 输入从不投递。
2. 文本、思考和工具活动映射为 `step/text` 和一个 `step/thinking` 块；无 `step/tool-use-start`。
3. 接受前和接受后的中断；被忽略的取消 → `uncertain` → 下一次调用初始化。
4. 可见输出之前的 Steer 替换请求；输出（含权限行）之后则成为新请求。
5. 接受前的暂时性失败被重试；接受后的失败不重试。
6. 子分支以及原生↔外部切换会初始化；父分支对话不受影响。
7. 带 `in_flight` 记录和待处理审批重启 → `uncertain`、`approval/cancelled`；不重发。
8. 对外部 profile 的 Collaboration 请求返回外部回复；位于其他节点的 leader 得到 `external_agent_wrong_node`。
9. 权限：WorkspaceWrite 下 `execute` → `approval/request`；allow → `allow_once`，deny → `reject_once`；ReadOnly 下 `execute` → 无事件直接 reject；FullAccess → 无事件直接 allow；无 `allow_once` 选项 → `cancelled`。
10. member Session：审批经 Collaboration 到达 leader，leader 的决定被投递；没有 leader 时由用户经 `approval.request` 回答。
11. 中断与 `approval.respond` 以两种顺序竞争；外部撤回取消审批；ExternalAgent 重载取消审批而不重载 ModelProvider。
12. 一个离线对等节点只使其自身的调用失败；其他节点和原生连接继续。
13. 共享 ACP 一致性测试套件在客户端侧通过。

## 附录 E. 假设

| # | 假设 | 依据 | 若被否决的影响 |
| --- | --- | --- | --- |
| A1 | 在加载和显式刷新时检测，而非监视 | 判断 | 后台轮询 |
| A2 | 每个外部 agent 一个模型 `default` | 判断 | 每个 agent 的模型列表 |
| A3 | 检测到的 agent 在启用前保持禁用 | 判断（它们消耗用户的账户） | 默认开启 |
| A4 | 取消宽限期 10 s | 判断 | 其他上限 |
| A5 | 每个对话一个 ACP 进程，空闲 10 分钟后关闭 | 判断 | 共享进程 |
| A6 | 适配器定义是插件内的数据 | Overview §F.3 | 每个 agent 一个插件 |
| A7 | ACP 工具类型 → 调用名 `external_agent.<kind>` 和风险类别映射 | Decisions E6（普通审批）；映射为判断 | 单一未分类类别（WorkspaceWrite 下总是询问） |
| A8 | 增量为空时调用失败 | 判断 | 发送继续 prompt |
| A9 | 外部工具活动渲染为思考行 | 判断 | 文本，或仅诊断信息 |
| A10 | 仅在适配器报告时提供用量 | 判断 | — |
| A11 | `max_turn_requests` 和 `refusal` 使请求失败 | 判断 | 映射为 `EndTurn` |
| A12 | 记录在最后使用后保留 30 天 | 判断 | 其他保留策略或在 Session 关闭时删除 |
| A13 | 外部 agent 只服务资源节点为其所在节点的 Session | Overview §6.3；判断 | 跨节点工作目录 |
| A14 | 转录格式；省略思考 | Overview §B.10 列出内容；格式为判断 | 其他渲染方式 |
| A15 | 不确定或失败的继续从历史重新初始化 | Overview §9（仍未决） | 显式失败 |
| A16 | CLI 适配器以 agent 自身的非自动批准模式启动；它们无法询问 dim | 判断（无请求通道） | 每个 CLI 的权限桥接 |
| A17 | 外部权限审批无超时 | Approval 没有超时；Questions 先例（E24） | 超时即以 reject 回答 |
| A18 | 审批调用输入 `{agent, title, kind, raw_input}`，`raw_input` 上限 4 KiB | 判断 | 其他形态 |

## 附录 F. 待对齐问题

| 问题 | 描述 |
| --- | --- |
| 补充的代价 | 准入 `approval_ask` 条目使 ModelProvider 依赖其分支的 Approval，因此 Approval 重载会重载 ModelProvider（罕见，按 D3 接受） |
| 外部对话的继续 | Overview §9；本设计选择重新初始化（A15） |
| Agents profile 注册 | Agents 的 profile 注册以及 Provider 的连接条目操作（直接依赖，ADR 002 D3）的形态尚未定义 |
| ACP `session/load` | 进程重启后，`ready` 记录是否应通过 `session/load` 继续，而不是初始化 |

## 附录 G. Rust 绑定

中立契约的一种绑定，用于 Rust 实现；TypeScript 实现通过其 rutis-bridge 运行时使用相同的名称和数据。

| 中立 | Rust |
| --- | --- |
| `modelKind` key `external-agent` | 在 `apply` 中向 ModelHost 注册、在 `ctx.effect` 中注销的 Provider；`invoke(Arc<ModelRequest>, ConnectionTarget, ModelCallScope, CancellationToken) -> HostFuture<ModelStream>`；审批询问为 `ModelChunk` 扩展条目；`answer_approval(request_id, ask_id, outcome)` |
| 模型块和失败数据 | `ModelChunk`、带 `ModelFailureFacts { origin: Transport, transient }` 的 `HostError::Failed`、`ModelOutputOutcome::Failed`（[host.rs](../../../../crates/dim-agent-capabilities/src/host.rs)） |
| ModelProvider 准入 | `B:Approval` `admit(AdmitCall { source: BranchSource::Host { operation_id }, call }, risk, cancel)` |
| `externalAgentNode` key `<node>` | 下方的 trait，由桥接行提供，经链路导出 |
| `sessions.history` | `AgentIndex` get，然后 `Session::local_page(branch_id, after, limit, through)` |
| `managementMethods` key `external_agents` | 由 Rpc 收集的 `Contributions<ManagementMethods>` 条目 |

```rust
trait ExternalAgentNode {
    async fn detect(&self) -> Vec<Detected>;
    async fn turn(&self, spec: TurnSpec, cancel: CancellationToken) -> Result<TurnStream, BridgeError>;
}
enum Conversation { New, Continue { external_id: String } }
enum TurnUpdate { Accepted { external_id: String }, Text(String), Thought(String),
                  ToolActivity { id: String, title: String, status: String },
                  PermissionAsked { ask_id: String, title: String, kind: String,
                                    raw_input: Option<JsonObject>, options: Vec<PermissionOption> },
                  PermissionWithdrawn { ask_id: String }, Usage(Usage), End(EndReason) }
impl TurnStream { fn answer(&self, ask_id: &str, answer: PermissionAnswer); } // Allow | Reject | Cancelled
```
