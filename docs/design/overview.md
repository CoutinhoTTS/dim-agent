# 总体设计：未设计功能的插件化

状态：草案

本文以 [CLI 功能清单](../feature-list.md) 为目标，找出现有插件职责尚未覆盖的功能，把它们设计成插件，并说明这些插件如何接入现有体系。每个新插件的详细需求另写在 `docs/design/plugins/<名称>.md`，按第 9 节的模板编写。

现有插件及其内容不在本文修改范围内。现有插件需要的补充与变更登记在第 6 节，由对应插件另行设计。

## 1. 范围与前提

| 项 | 约定 |
| --- | --- |
| 终端形态 | 只设计 scrollback。功能清单中标为“仅全屏 TUI”的项不在范围内 |
| 多模态 | 只设计图片。功能清单中“仅限特定 Desktop 构建”的命令不在范围内 |
| 动态事件 | 以 [#2](https://github.com/arcships/dim/issues/2) 已完成为前提：插件在运行时注册自己的事件类型（类型名、数据格式），注册随插件生命周期，Timeline 按注册校验写入。新插件不向内核事件枚举增加类型，也不设过渡方案 |
| 模型上下文 | 按 [ADR 001](../adr/001-harness-foundation.md) 的提议：插件事件不直接投影进模型上下文；给模型看的内容经采用的输入或工具结果进入。#2 中“由注册的投影规则决定”的方案与此不同，见第 11 节 |
| 配置与插件管理 | 依赖 [#3](https://github.com/arcships/dim/issues/3) 的部分按 ADR 001 的“按来源写入、再发布到根 loader”方案设计，并逐处标注 |
| 现有设计 | [时间总线](../architecture/rust-agent/time-bus.md)、[设计规则](../architecture/rust-agent/principles.md)、[单树装配](../architecture/rust-agent/single-tree.md)、[生命周期](../architecture/rust-agent/lifecycle.md) 与 ADR 001 的信任合同对所有新插件同样适用 |

## 2. 现有体系

| 层 | 职责 | 现有插件（按职责分组） |
| --- | --- | --- |
| CLI | 终端交互、命令、渲染；不执行 Agent | `dim-cli` |
| App Server 进程 | 协议与连接、Session 目录与持久化、配置来源、Provider 与账号、MCP、Skill 目录、协作目录 | Transport、Connection、Rpc、ServerControl；AgentDirectory、Agents、SessionStore、SqliteStorage；Settings；Provider、ManagedProviders、各 OAuth、Aimux；McpPublication、Mcp；SkillsCatalog；Collaboration、CollaborationDirectory；FileStorage、FileWriteCoordinator、HostPlatform |
| Session | 工具注册、提示词来源、分支树 | ToolRegistry、PromptRegistry、PromptSources、Skills、Wait、WorkspaceTools、TimelineTree |
| 分支 | 输入、循环、上下文、模型、工具执行、准入 | InputQueue、Loop、Prompt、ContextPreparation、ModelProvider、ToolBinding、ToolExecutor、Approval、Permission、BackgroundExec、FileReads、WaitBranch、SkillsBranch、PromptSourcesBranch、TreeTimeline |

各插件的职责见 [plugins](../architecture/rust-agent/plugins/README.md) 与 [单树装配](../architecture/rust-agent/single-tree.md) 的进程插件表。

## 3. 接入方式

新插件只通过以下五种方式接入，不修改内核：

| 方式 | 机制 | 现有先例 |
| --- | --- | --- |
| **A 贡献工具** | Session 插件向 ToolRegistry 贡献工具；分支经 ToolBinding 采用并固定处理器；ToolExecutor 执行；Approval 与 Permission 准入。模型只读工具结果的 `content` | WorkspaceTools、Skills、Wait、TimelineTree |
| **B 输入注入** | 插件把最终内容写成 `input/message`，按模式（Steer、Append、Queue）由 InputQueue 采用、Loop 消费后进入上下文 | BackgroundExec 完成通知、协作回报 |
| **C 提示词来源** | 向 PromptRegistry 注册命名来源；分支由 PromptSourcesBranch 发现变化并经 Prompt 采用 | AGENTS.md、Skill 列表、环境与日期 |
| **D 脱离工作** | 发起它的插件自己执行、取消、恢复和结算；结果写在发起分支；需要时以 B 唤醒 | BackgroundExec、Collaboration |
| **E 进程服务** | App Server 进程插件；配置经 Settings 按来源写入；对外提供管理 RPC；外部协议适配与 Rpc 平级 | Mcp、Provider、SkillsCatalog |

共同约束：

- 插件的事件由插件自己声明，类型名为 `领域/名称`。插件只写自己声明的事件；发起工作的插件写该工作的终态（时间总线第 2 条）。
- 中断时插件取消自己的在途工作、释放资源后再写终态（[中断](../architecture/rust-agent/interrupt.md)）。
- 进程级状态（配置、作业存储、凭据引用）不写入 Timeline；影响分支后续行为的结果才写入。
- 管理类 RPC 只对受保护的本地连接开放，WebSocket 远程调用方不可用，与 ADR 001 的 MCP 管理方法一致。

## 4. 功能归属

“现有”表示由现有插件职责覆盖；“扩展”表示属于现有插件职责但现有设计未写到，见第 6 节；“新”表示第 5 节的新插件；“CLI”表示只在 CLI 层实现，见第 7 节；“范围外”见第 1 节。

### Provider 与模型

| 功能 | 归属 |
| --- | --- |
| OAuth / API-key Provider、Provider 管理、`/connect`、`/models` | 现有：Provider、ManagedProviders、各 OAuth、Aimux |
| `/effort`、`/fast` | 扩展：Prompt、ModelProvider |
| 子代理模型、辅助模型 | 新：ModelPolicy |

### Session 管理

| 功能 | 归属 |
| --- | --- |
| `/new`、`/resume`、Session 列表、`/history` | 现有：Agents、SessionStore |
| `/timeline` | CLI（读取历史） |
| Session 标题 | 新：SessionTitle |
| `/export`、Session 分叉、删除与清理、请求快照导出 | 新：SessionArchive |
| 文件回滚、文件与对话回滚 | 范围外（仅全屏 TUI） |

### 会话内交互

| 功能 | 归属 |
| --- | --- |
| 发送、排队、自动发送、Steer、中断 | 现有：InputQueue、Loop |
| `Ctrl+S`、`/queue`、取回编辑、删除排队消息 | 扩展：InputQueue |
| 多行编辑、输入历史、命令补全、Skill 与文件引用、粘贴、剪贴板、复制、打开链接 | CLI |
| 图片附件、大段粘贴转文本附件、混合附件 | 新：Attachments |
| `!command`、`!!command` | 新：ShellInput |
| 流式输出、运行状态、文件变更反馈 | 现有事件，CLI 渲染 |
| 工具审批 | 现有：Approval、Permission |
| 用户提问 | 新：Questions |
| Todo 状态 | 新：Todo |
| 后台任务查看与停止 | 扩展：BackgroundExec、Collaboration；Workflow 与 ExternalAgent 各自提供 |

### 系统管理

| 功能 | 归属 |
| --- | --- |
| `/exit`、`/theme`、`/keys`、升级、`/tui` 以外的终端设置 | CLI |
| `/settings` | 各项归属对应插件；CLI 汇总入口 |
| `/usage` | 扩展：ContextPreparation（上下文用量）、ManagedProviders（套餐用量） |
| `/language` | CLI（界面）；扩展：PromptSources（提示词语言） |
| `/debug`、调试日志 | 现有：Trace 诊断通道 |
| `/stats`、鼠标设置 | 范围外（仅全屏 TUI） |

### Agent 行为

| 功能 | 归属 |
| --- | --- |
| `/agent` | 扩展：Agents、Prompt |
| `/goal`、目标继续 | 新：Goal |
| `/approvals`、命令规则 | 扩展：Permission、Settings |
| 权限策略三档 | 现有：Permission |
| 沙箱 | 新：Sandbox |
| `/compact`、自动压缩 | 现有：ContextPreparation（压缩宿主为现有合同） |
| `/scheduled`、定时任务 | 新：Scheduler |

### 扩展

| 功能 | 归属 |
| --- | --- |
| `/plugins`、`/reload-plugins`、插件市场、插件安装 | 新：PluginManager |
| `/skills` 查看与启用、`/reload-skills`、Skill 来源 | 现有：SkillsCatalog、Skills；安装与移除归 PluginManager |
| `plan` Skill、`AGENTS.md` | 现有：Skill 内容、PromptSources |
| `/mcp`、MCP 连接 | 现有：Mcp、Settings；HTTP/SSE 的 OAuth 授权为扩展 |
| `/workflow`、`/workflow:save`、工作流管理 | 新：Workflow |
| `/external-agents`、外部 Agent | 新：ExternalAgent |
| 并行子代理 | 现有：Collaboration；模型选择归 ModelPolicy |
| Hooks | 新：Hooks |
| 网页搜索与抓取 | 新：Web |
| Slack 与飞书 | 新：Notifications |

### 工作区工具

`read`、`write`、`edit`、`glob`、`grep`、`exec`、`wait` 均由现有 WorkspaceTools、BackgroundExec、Wait 覆盖。

### 远程与外部访问

| 功能 | 归属 |
| --- | --- |
| JSON-RPC 常驻服务 | 现有：Transport、Rpc |
| ACP | 新：AcpGateway |
| `/web`、Web 访问、`/remote`、远程服务、手机配对、远程 Session | 新：RemoteGateway |
| VS Code | CLI |

### CLI 命令

子命令体系、`exec --json`、`exec --verify` 属于 CLI；各子命令调用对应插件的 RPC。

### 多模态

| 功能 | 归属 |
| --- | --- |
| `image generate`、`image edit`、`image enhance`、`modality`、`modality task` | 新：Modality |
| `image read` | 现有：视觉模型经 `read` 读取图片 |
| 视频、语音、音乐、音频、3D、动作、实时、OCR | 范围外（仅 Desktop） |

## 5. 新插件

每个插件给出作用域、职责、接入方式、事件、对外接口和与现有插件的关系。事件与接口名为设计建议，以各插件需求文档为准。

### 5.1 Questions

| 项 | 设计 |
| --- | --- |
| 作用域 | Session（工具贡献）+ 分支（待答问题状态） |
| 职责 | 模型向用户提问并等待回答：单选、多选、多问题、自定义回答 |
| 接入 | A：工具 `ask_user`，回答作为工具结果返回 |
| 事件 | `question/asked {question_id, call_id, questions}`、`question/answered {question_id, answers}`、`question/cancelled {question_id, reason}` |
| 接口 | RPC `session.question.respond`；Session 事件推送待答问题 |
| 关系 | 流程与 Approval 相同：写请求、等回答、中断时以 `question/cancelled` 结算并返回取消的工具结果。非交互运行（`exec`）没有回答方时立即以取消结算 |

### 5.2 Todo

| 项 | 设计 |
| --- | --- |
| 作用域 | Session（工具贡献）+ 分支（列表状态） |
| 职责 | 记录模型的任务拆解、当前项与完成状态 |
| 接入 | A：工具 `todo_write` 整体替换列表；工具结果 `content` 返回当前列表 |
| 事件 | `todo/updated {items: [{id, text, status}]}` |
| 接口 | 分支只读状态供 CLI 显示 |
| 关系 | 状态由 `todo/updated` 重建；子分支各自持有列表，不继承父分支状态 |

### 5.3 Goal

| 项 | 设计 |
| --- | --- |
| 作用域 | Session（工具贡献）+ 分支（目标状态） |
| 职责 | 设置、编辑、暂停、恢复、清除目标；目标未完成时自动继续；记录完成或受阻 |
| 接入 | A：工具 `goal_update` 报告完成或受阻；B：继续时写入 Queue 模式的继续输入 |
| 事件 | `goal/set`、`goal/paused`、`goal/resumed`、`goal/cleared`、`goal/completed {summary}`、`goal/blocked {reason}` |
| 接口 | RPC `session.goal.set/pause/resume/clear` |
| 关系 | 订阅 `turn/end`：目标处于活动状态、turn 正常结束且没有待消费输入时，写继续输入驱动下一 turn，不修改 Loop。`stop/interrupt` 使目标暂停；连续自动继续次数有上限，达到后暂停并写 `goal/paused` |

### 5.4 ShellInput

| 项 | 设计 |
| --- | --- |
| 作用域 | 分支 |
| 职责 | 执行用户在输入框发起的 `!command` 与 `!!command` |
| 接入 | D：命令作为本插件的工作执行；B：`!command` 结束后把命令与有界输出写成 Append 模式输入，在下一次模型请求时进入上下文，不单独开始 turn |
| 事件 | `shell/started {run_id, command, cwd, context}`、`shell/ended {run_id, exit_code, output, truncated}` |
| 接口 | RPC `session.shell.run`、`session.shell.stop` |
| 关系 | 经 HostPlatform 启动进程，受 Sandbox 约束；用户直接发起，不经 Approval。`!!command` 只写事件，不写输入 |

### 5.5 Attachments

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程（附件存储）+ Session（附件归属） |
| 职责 | 保存图片附件与文本附件，供输入引用 |
| 接入 | B：`input/message` 的内容引用附件 ID；图片附件在模型请求中成为图片输入，文本附件以文件路径出现，由模型按需 `read` |
| 事件 | 无独立事件；附件元数据随输入记录 |
| 接口 | RPC `attachment.put` 返回附件 ID；Session 删除时清理其附件 |
| 关系 | 经 FileStorage 写入 Session 目录。输入内容引用附件属于输入数据类型的扩展，见第 6 节。模型不支持图片时在输入边界拒绝并给出明确错误 |

### 5.6 Web

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程（搜索服务配置）+ Session（工具贡献） |
| 职责 | 网页搜索与网页正文抓取 |
| 接入 | A：工具 `web_search`、`web_fetch`；抓取结果转为有界正文 |
| 事件 | 无；结果即工具结果 |
| 接口 | 搜索服务与凭据引用经 Settings 配置 |
| 关系 | 工具风险类别为“网络”，需要 Permission 增加该类别，见第 6 节 |

### 5.7 ModelPolicy

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程 |
| 职责 | 为辅助用途选择模型：子代理、辅助任务（标题、压缩、摘要）、默认多模态模型 |
| 接入 | E：策略为“继承主 Session / 固定模型与推理强度 / 由 Agent 选择（仅子代理）” |
| 事件 | 无；策略是配置 |
| 接口 | 服务 `ModelPolicy::resolve(role, session)`；RPC 管理策略 |
| 关系 | 经 ModelSelectionHost 解析连接；Collaboration、SessionTitle、压缩宿主、Modality 使用它 |

### 5.8 SessionTitle

| 项 | 设计 |
| --- | --- |
| 作用域 | Session |
| 职责 | 自动生成 Session 标题；支持手动改名 |
| 接入 | 订阅主分支第一个正常结束的 `turn/end`，用 ModelPolicy 的辅助模型生成标题 |
| 事件 | `session/titled {title, source: auto \| manual}` |
| 接口 | RPC `session.rename`；Session 列表投影标题 |
| 关系 | 手动标题优先，自动生成不覆盖手动标题。生成失败不影响 Session |

### 5.9 Hooks

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程（Hook 目录与审查）+ 分支（执行） |
| 职责 | 在生命周期节点执行命令 Hook：Session 开始、用户提交、工具调用前、工具调用后、turn 结束、通知 |
| 接入 | E：Hook 定义来自用户、项目、插件三种来源；观察类 Hook 订阅事件；工具调用前的 Hook 参与准入；Hook 要求注入的输出经 B 写入 |
| 事件 | `hook/started {hook_id, point}`、`hook/ended {hook_id, outcome, output, truncated}` |
| 接口 | RPC `hooks.catalog`、`hooks.approve`、`hooks.set_enabled`、`hooks.disable_all` |
| 关系 | 项目来源的 Hook 与项目 MCP 一样，需要宿主本地批准才能执行，批准绑定配置指纹。工具调用前的 Hook 需要 Approval 提供准入扩展点，见第 6 节。插件来源依赖 #3 |

### 5.10 Scheduler

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程（作业与触发）+ Session（工具贡献） |
| 职责 | 定时任务的创建、编辑、暂停、恢复、删除、立即运行与执行历史 |
| 接入 | E：到点时经 Agents 新建或复用 Session，提交作业提示词；A：工具 `schedule` 供模型管理作业 |
| 事件 | 作业与执行历史存于进程存储，不写 Timeline；被触发的 Session 收到的输入标明来源为定时作业 |
| 接口 | RPC `schedule.list/save/delete/set_enabled/run_now/history` |
| 关系 | 作业字段：周期、提示词、工作目录、模型、新建或复用 Session。触发要求 daemon 存活；错过的时间窗跳过，不补跑。模型经工具新建或修改作业属于持久配置变更，需经 Approval |

### 5.11 Workflow

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程（工作流定义）+ Session（运行） |
| 职责 | 查看工作流、提供参数并启动；查看阶段、日志、状态与结果；停止；保存工作流供复用 |
| 接入 | D：一次运行是 Workflow 的脱离工作，各阶段在子分支（TimelineTree）或协作成员（Collaboration）中执行；B：运行结束以通知唤醒发起分支；A：工具 `workflow` |
| 事件 | `workflow/started`、`workflow/stage_started`、`workflow/stage_ended`、`workflow/ended {outcome}` |
| 接口 | RPC `workflow.list/save/start/status/stop` |
| 关系 | 定义文件经 Settings 的用户与项目来源保存；阶段模型经 ModelPolicy 选择 |

### 5.12 ExternalAgent

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程（探测与配置）+ Session（运行） |
| 职责 | 把本机已安装的 Claude Code、Codex、Cursor、OpenCode 等作为外部执行者，委派任务、查看进度、取消 |
| 接入 | D：每次委派是脱离工作，经 HostPlatform 以非交互方式启动，支持 ACP 的外部 Agent 经 ACP 客户端连接；B：结束后以通知回报；A：工具 `external_agent` |
| 事件 | `external_agent/started {run_id, agent, cwd}`、`external_agent/progress`（有界）、`external_agent/ended {run_id, outcome, summary}` |
| 接口 | RPC `external_agents.list/set_enabled` |
| 关系 | 启动外部进程的风险类别为执行，经 Approval；可配置受 Sandbox 约束 |

### 5.13 PluginManager

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程 |
| 职责 | 插件市场（Git 来源）的增删与更新；搜索、查看、安装（市场、本地路径、Git URL）、启用、停用、重载、移除插件 |
| 接入 | E：插件清单声明其内容：Skill、Hook、MCP 配置、工作流、能力插件；安装即写入来源并发布 loader 行 |
| 事件 | 无 Timeline 事件；状态来自 loader 条目 |
| 接口 | RPC `plugin.marketplace.*`、`plugin.install/remove/set_enabled/reload/list` |
| 关系 | 依赖 #3（loader 管理）；Rust 以外语言的能力插件依赖 [#4](https://github.com/arcships/dim/issues/4)。插件内容按来源接入 SkillsCatalog、Hooks、Mcp、Workflow。项目来源的插件需要宿主本地批准 |

### 5.14 Sandbox

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程 |
| 职责 | 为进程执行提供操作系统级隔离：文件、网络、进程 |
| 接入 | HostPlatform 启动进程时套用隔离配置 |
| 事件 | 无；被拒绝的操作体现在调用方的结果中 |
| 接口 | 配置经 Settings；平台不支持时明确报告 |
| 关系 | Permission 三档预设分别对应只读、工作区可写、完全访问三种隔离配置。适用于 `exec`、ShellInput、Hooks，ExternalAgent 可配置。平台后端：macOS seatbelt、Linux landlock 或 bubblewrap；Windows 无后端时明确不可用。准入（Permission）与隔离（Sandbox）是两层，互不替代 |

### 5.15 Notifications

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程 |
| 职责 | 任务完成、待审批、待回答、定时作业结束时发送通知；发送 Slack 与飞书消息 |
| 接入 | E：订阅 Session 广播；A：工具 `notify` 供模型主动发送 |
| 事件 | 无 Timeline 事件；发送记录存于进程诊断 |
| 接口 | 渠道与凭据引用经 Settings 配置 |
| 关系 | 工具发送是对外发消息，经 Approval。终端内提醒由 CLI 完成，不经本插件 |

### 5.16 SessionArchive

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程 |
| 职责 | 导出对话（Markdown、JSON）；删除 Session；按时间或保留数量清理，先预览；从历史消息分叉出新 Session；导出模型请求快照 |
| 接入 | E |
| 事件 | 新 Session 记录分叉来源；其余为进程操作 |
| 接口 | RPC `session.export`、`session.delete`、`session.cleanup`（含预览）、`session.fork`、`session.request_export` |
| 关系 | 与 SessionStore、Agents 配合。删除正在运行或被连接的 Session 时拒绝。分叉复制来源截点之前的事实并记录来源，不重放副作用，语义与分支截点一致 |

### 5.17 AcpGateway

| 项 | 设计 |
| --- | --- |
| 作用域 | CLI 进程 |
| 职责 | `dimcode acp` 在标准输入输出上提供 ACP 服务，供编辑器连接 |
| 接入 | 协议适配：ACP 会话对应 dim Session；提示对应提交；更新来自 Session 事件；权限请求对应 `approval.request` |
| 事件 | 无 |
| 接口 | ACP 协议 |
| 关系 | 与 CLI 一样经本地 RPC 连接 daemon，不在 daemon 中另开执行路径 |

### 5.18 RemoteGateway

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程 |
| 职责 | Web 工作区与远程访问：启动与停止服务、认证、访问令牌轮换、手机配对（二维码或配对码）、设备授权与撤销、远程访问 Session |
| 接入 | E：建在 Transport 的 WebSocket 之上，增加认证、配对与设备授权；提供 Web 工作区静态资源 |
| 事件 | 无 Timeline 事件；设备与令牌存于进程存储 |
| 接口 | RPC `remote.start/stop/status/pair/devices/revoke/rotate_token` |
| 关系 | 远程客户端只能使用 Session 级方法，不能使用管理方法。认证、断线与 TLS 部署的取舍与 [#5](https://github.com/arcships/dim/issues/5) 的待定问题一并决定 |

### 5.19 Modality

| 项 | 设计 |
| --- | --- |
| 作用域 | 进程 |
| 职责 | 图片生成、编辑（参考图与遮罩）、增强；异步任务的状态查询、等待与取消；查看能力与默认模型 |
| 接入 | E：经 Provider 调用非对话类模型 |
| 事件 | 无 Timeline 事件；任务存于进程存储 |
| 接口 | RPC `modality.capabilities`、`modality.task.*`；CLI `image`、`modality` 子命令 |
| 关系 | 需要 Provider 支持非对话类模型，见第 6 节；默认模型经 ModelPolicy |

## 6. 现有插件的补充与变更

现有插件需要两类改动：一类是功能清单中属于其职责、但现有设计未覆盖的功能；另一类是新插件接入时需要现有插件提供的配合。本文只登记，具体设计写入对应插件的需求文档。

- **补充**：新增服务、方法或配置项，原有行为不变。
- **变更**：原有行为或合同改变，需要说明兼容与迁移。

所有现有事件类型改为经 #2 的注册机制声明，属于 #2 本身的工作，不在下表重复。

### 6.1 分支与 Session 能力

| 插件 | 类型 | 内容 | 原因 |
| --- | --- | --- | --- |
| 输入数据类型（`input/message`） | 补充 | 内容可引用附件 ID；输入携带来源标记（用户、定时作业、目标继续、Shell、Hook、通知） | Attachments；Scheduler、Goal、ShellInput、Hooks 需要区分来源，CLI 据此显示 |
| InputQueue | 补充 | 删除尚未采用的排队消息；把排队消息转为 Steer；队列状态包含可删除性 | `Ctrl+S`、`/queue`、取回编辑、删除排队消息 |
| Permission | 补充 | “网络”风险类别；来自 Settings 的跨 Session 规则来源；允许命令清单 | Web；`/approvals`、命令规则 |
| Permission | 变更 | 三档预设除准入外还决定 Sandbox 隔离配置 | Sandbox |
| Approval | 补充 | 工具调用前的准入扩展点：已登记的准入方（如 Hooks）在策略判定后、执行前给出允许或拒绝 | Hooks |
| Prompt、ModelProvider | 补充 | 模型参数（推理强度、快速模式）随模型配置采用，并传入模型请求 | `/effort`、`/fast` |
| Prompt | 补充 | 在 Session 内切换 Agent 配置（提示词、模型、工具集）作为一次配置采用 | `/agent` |
| PromptSources | 补充 | 提示词语言来源 | `/language` |
| ContextPreparation | 补充 | 查询当前上下文用量与预算 | `/usage` |
| ContextPreparation | 变更 | 生产用压缩宿主经 ModelPolicy 的辅助模型执行 | ModelPolicy |
| BackgroundExec | 补充 | 列出本 Session 在途工作并支持全部停止的查询与停止接口；Workflow、ExternalAgent 以同一形式登记 | 后台任务 |
| HostPlatform | 变更 | 启动进程时按调用方作用域套用 Sandbox 隔离配置；平台不支持时明确报告 | Sandbox |

### 6.2 App Server 进程插件

| 插件 | 类型 | 内容 | 原因 |
| --- | --- | --- | --- |
| Settings | 补充 | 新的配置来源：权限规则、Hook 定义与 Hook 批准、工作流定义、Web 搜索服务、通知渠道、模型策略、沙箱配置、插件市场；统一的凭据引用 | Hooks、Workflow、Web、Notifications、ModelPolicy、Sandbox、PluginManager、Permission 规则 |
| Rpc | 补充 | 第 5 节各插件的 Session 级与管理级方法；方法按级别分组，供网关判断可用范围 | 全部新插件；RemoteGateway |
| Transport、Connection | 变更 | 远程连接携带认证后的设备身份；按身份限制可用方法 | RemoteGateway |
| Agents | 补充 | 以指定来源新建或复用 Session 并提交首条输入；从另一 Session 的截点创建 Session | Scheduler；SessionArchive 分叉 |
| SessionStore | 补充 | Session 删除与按时间或数量清理（含预览）；Session 列表投影标题；记录分叉来源；删除时清理附件 | SessionArchive、SessionTitle、Attachments |
| Collaboration | 变更 | 子代理模型改由 ModelPolicy 选择 | ModelPolicy |
| Collaboration | 补充 | 在途成员纳入统一的在途工作查询与停止 | 后台任务 |
| Provider | 补充 | 非对话类模型（图片） | Modality |
| Provider | 补充 | 按用途解析模型连接，供 ModelPolicy 使用 | ModelPolicy |
| ManagedProviders | 补充 | 查询套餐用量；未提供时明确不可用 | `/usage` |
| Mcp | 补充 | HTTP/SSE 连接的 OAuth 授权；接受插件来源的配置 | MCP 连接；PluginManager |
| SkillsCatalog | 补充 | 接受插件来源的 Skill 根；安装与移除由 PluginManager 发起 | PluginManager |
| Trace | 补充 | 导出指定步骤的模型请求快照，可过滤推理、工具记录与压缩内容 | 请求快照导出 |
| Session 广播 | 待确认 | 进程级订阅所有 Session 的结束、待审批、待回答等事件 | Notifications |

## 7. CLI 层

以下只在 CLI 中实现，不新增插件：多行编辑、输入历史、命令补全、`$` 与 `@` 引用、粘贴与拖放路径、剪贴板、消息复制、打开链接、`/timeline`、`/exit`、`/theme`、`/keys`、界面语言、升级、VS Code 集成、子命令体系、`exec --json`、`exec --verify` 修复循环。

## 8. 横切约定

| 主题 | 约定 |
| --- | --- |
| 事件 | 每个插件在需求文档中列出声明的事件、数据格式与消费规则（哪些状态机必须消费、消费后达到什么效果） |
| 上下文 | 只经 A 或 B 进入模型上下文；持续生效的规则经 C |
| 配置 | 每项配置有明确来源（用户、工作区、Session）和唯一写入路径；凭据只以引用形式出现在配置中 |
| 信任 | 来自项目文件的可执行内容（Hook、MCP、插件）需要宿主本地批准，批准绑定配置指纹；读取配置不产生执行权限；回放不恢复权限 |
| 持久化 | 进程级存储变更遵循现有存储兼容策略；不跨不兼容格式复用版本号 |
| 协议 | 新 RPC 方法区分 Session 级与管理级；管理级只对受保护的本地连接开放 |
| 国际化 | 用户可见文案由 CLI 负责；模型可见文案由提示词来源负责 |

## 9. 插件需求文档模板

每个新插件的需求文档 `docs/design/plugins/<名称>.md` 包含：

1. 用户流程：从用户或模型的操作到可见结果
2. 范围：负责与不负责的内容
3. 作用域与依赖：作用域、注入的服务、提供的服务
4. 事件：类型名、数据格式、写入方、消费规则
5. 状态：拥有的状态及如何由 Timeline 或进程存储重建
6. 接口：工具、RPC、CLI 入口
7. 错误与恢复：取消、中断、重载、进程退出时的结算
8. 安全：准入、信任、隔离、敏感数据
9. 验收：需要的流程测试

## 10. 实施阶段

| 阶段 | 内容 |
| --- | --- |
| P0 地基 | #2 动态事件；新 RPC 方法的分级约定 |
| P1 核心交互 | Questions、Todo、ShellInput、Attachments、ModelPolicy、SessionTitle、SessionArchive；扩展 InputQueue、Permission 规则、模型参数；CLI 子命令与 `exec --json` |
| P2 能力扩展 | Goal、Web、Sandbox、Hooks（用户与项目来源）、Scheduler、Workflow、Notifications；扩展 Mcp OAuth |
| P3 外部集成 | ExternalAgent、AcpGateway、RemoteGateway、Modality；PluginManager（依赖 #3、#4） |

## 11. 待对齐问题

| 问题 | 说明 |
| --- | --- |
| 事件进入上下文的方式 | #2 已定“由注册的投影规则决定”，ADR 001 提议“只经输入与工具结果”。本文按后者设计，需维护者对齐 |
| 定时任务的存活条件 | 功能清单写“交互终端进程存活”，本文按 daemon 存活设计 |
| 远程访问 | 认证、断线与 TLS 部署与 #5 的待定问题相关 |
| Sandbox 的覆盖面 | MCP stdio 进程与 ExternalAgent 是否默认受隔离 |
