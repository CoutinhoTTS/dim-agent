# Rust 会话存储格式

当前格式为 v13。v13 新增用户配置来源表 `settings_documents`；v12 的独立可丢弃恢复快照 `timeline_snapshots` 保持不变。严格拒绝 v12 及更旧数据库，保留原文件，不自动迁移。managed token 只存在所属 OAuth 文件记录，不进入 SQLite。静态 Provider 凭据及 MCP 私有配置只能由所属服务读取，不进入目录投影、Session 或 Timeline。开发验证使用新的数据库路径。存储版本与 RPC 协议版本独立。

## 当前 v13 表结构

下表逐列对应 `SqliteStorage::SCHEMA`。新表按业务实体命名，其中 `agent_sessions` 与旧 TypeScript 的 `sessions` 区分；当前启动门禁仍只接受本格式，不因此支持旧表同库存取。`—` 表示 SQL 中没有显式 `DEFAULT`；可空列若省略会得到 SQLite 的隐式 `NULL`。SQLite rowid 表中的 `TEXT PRIMARY KEY` 不自动保证非空，因此下表如实标出 v1 的两个缺口；正常写入仍必须给出 ID。

| 表.列 | SQLite 类型 | 可空 | 默认 | 约束与含义 |
| --- | --- | --- | --- | --- |
| `agent_store_format.version` | `INTEGER` | 否 | — | 主键，`CHECK(version = 13)`；数据库恰有一行是启动门禁 |
| `settings_documents.source` | `TEXT` | 否 | — | 主键；当前为 `user/mcp` 或 `user/skills`；工作区文件不镜像进此表 |
| `settings_documents.version` | `INTEGER` | 否 | — | 正整数，来源独立的 CAS 修订号；RPC token 为 `native:<source>:<version>` |
| `settings_documents.document` | `TEXT` | 否 | — | 对应 `McpSettings` 或 `SkillSettings` JSON，保留未知扩展字段；私有数据，不直接序列化到管理面 |
| `settings_documents.imported_revision` | `TEXT` | 否 | — | 首次导入原文件的 SHA-256，原文件缺失为 `missing`；仅供来源追溯 |
| `provider_imports.kind` | `TEXT` | 否 | — | 联合主键；`provider` 或 `selection` |
| `provider_imports.id` | `TEXT` | 否 | — | 联合主键；来源业务 ID，删除 Provider 不删除标记 |
| `provider_imports.source_schema` | `TEXT` | 否 | — | 已检查的来源两表结构 SHA-256 |
| `provider_imports.source_version` | `INTEGER` | 否 | — | 来源行版本，不作目标 CAS 版本 |
| `provider_imports.source_digest` | `TEXT` | 否 | — | 来源完整行 SHA-256，不包含源文件路径，不通过 RPC 返回 |
| `providers.provider_id` | `TEXT` | 否 | — | 主键；managed、bundled 与 custom 共用 |
| `providers.version` | `INTEGER` | 否 | — | 正整数，目录与启用状态的 CAS 版本 |
| `providers.record` | `TEXT` | 否 | — | authority-specific `ProviderRecord` JSON；managed 记录不含凭据，静态记录可含请求凭据且只允许非秘密投影读取 |
| `provider_selections.scope` | `TEXT` | 否 | — | 主键，`global` 或 `workspace:<absolute cwd>` |
| `provider_selections.version` | `INTEGER` | 否 | — | 正整数，选择的 CAS 版本 |
| `provider_selections.provider_id` | `TEXT` | 否 | — | Provider ID；与 model_id 共同标识目标 |
| `provider_selections.model_id` | `TEXT` | 否 | — | 对应 Provider 的模型 ID；模型暂不可用时保留选择 |
| `agents.agent_id` | `TEXT` | **SQL 允许** | — | 主键；正常写入必须是非空 Agent ID |
| `agents.profile` | `TEXT` | 否 | — | `AgentProfile` JSON，创建后固定 |
| `agents.is_default` | `INTEGER` | 否 | — | `CHECK(is_default IN (0, 1))`；唯一的 1 由 `agents_default_unique` 保证至多一个，至少一个由目录校验保证 |
| `agent_sessions.session_id` | `TEXT` | 否 | — | 主键，原 Session ID |
| `agent_sessions.created_at_ms` | `INTEGER` | 否 | — | `CHECK(created_at_ms >= 0)`；创建事务写入真实 Unix 毫秒 |
| `agent_sessions.config` | `TEXT` | 否 | — | `SessionConfig` JSON，创建时的配置 |
| `session_owners.session_id` | `TEXT` | **SQL 允许** | — | 主键，外键到 `agent_sessions(session_id)`；业务要求每个 Session 恰有一条非空 owner |
| `session_owners.agent_id` | `TEXT` | 否 | — | 外键到 `agents(agent_id)` |
| `session_owners.protocol_version` | `INTEGER` | 否 | — | `CHECK(protocol_version IN (1, 2, 3))`；Session 所属协议版本 |
| `timeline_branches.session_id` | `TEXT` | 否 | — | 联合主键第 1 列；外键到 `agent_sessions(session_id)` |
| `timeline_branches.branch_id` | `TEXT` | 否 | — | 联合主键第 2 列；同 Session 内唯一 |
| `timeline_branches.info` | `TEXT` | 否 | — | `TimelineInfo` JSON：`id`、可选 `parent`、`branch_point`、`create_anchor` |
| `timeline_branches.sequence` | `INTEGER` | 否 | — | `CHECK(sequence >= 0)`；本分支最新已提交事件序号，派生水位 |
| `timeline_content.session_id` | `TEXT` | 否 | — | 联合主键第 1 列；外键到 `agent_sessions(session_id)` |
| `timeline_content.hash` | `TEXT` | 否 | — | 联合主键第 2 列；同 Session 内内容地址 |
| `timeline_content.kind` | `TEXT` | 否 | — | `message`、`tools` 或 `system`；v1 无 SQL 枚举 `CHECK`，读取时校验 |
| `timeline_content.body` | `TEXT` | 否 | — | 对应类型的完整 JSON 正文；写入后不可原地修改 |
| `timeline_events.session_id` | `TEXT` | 否 | — | 联合主键第 1 列；与 `branch_id` 一起外键到 `timeline_branches` |
| `timeline_events.branch_id` | `TEXT` | 否 | — | 联合主键第 2 列 |
| `timeline_events.sequence` | `INTEGER` | 否 | — | 联合主键第 3 列，`CHECK(sequence > 0)`；物理行首事件的本分支序号 |
| `timeline_events.last_sequence` | `INTEGER` | 否 | — | `CHECK(last_sequence >= sequence)`；物理行末事件序号，合并行可大于 `sequence` |
| `timeline_events.kind` | `TEXT` | 否 | — | `EventKind::key()`，如 `model/call`、`step/text`；与正文逐事件核对 |
| `timeline_events.body` | `TEXT` | 否 | — | 下节定义的 `Row` JSON，包含事件、引用、before 轨迹及批次终点 |
| `timeline_events.event_id` | `TEXT` | 是 | 隐式 `NULL` | 普通行保存原事件 ID；输出合并行为空；恢复时核对正文 |
| `timeline_snapshots.session_id` | `TEXT` | 否 | — | 联合主键第 1 列；与 branch_id、sequence 一起外键到事件锚点 |
| `timeline_snapshots.branch_id` | `TEXT` | 否 | — | 联合主键第 2 列 |
| `timeline_snapshots.sequence` | `INTEGER` | 否 | — | 联合主键第 4 列，`CHECK(sequence > 0)`；已提交 completed 压缩事件序号 |
| `timeline_snapshots.interpreter` | `TEXT` | 否 | — | 联合主键第 3 列；解码前可检查的解释器兼容合同 |
| `timeline_snapshots.event_id` | `TEXT` | 否 | — | 锚点的精确事件 ID，写入和候选查询校验 |
| `timeline_snapshots.state` | `TEXT` | 否 | — | 不透明原始字符串；SQLite 不解析或反序列化 |

v1 的外键只约束上述表列；`info.parent`、`body` 中的内容哈希、before 轨迹和检查点位置都是 JSON 内关系，必须在存储读取边界校验。`timeline_content` 的地址是小写十六进制 `SHA-256(kind + NUL + body)`，其中 `body` 是写入的原始 JSON 字符串字节；同一内容只在同一 Session 内共享。创建 Session、owner 和 `main` 分支是同一事务；写入轮把事件、首次出现的内容、由 `branch/create` 生成的分支目录及分支 head 放在同一事务，提交后才发布。

## 持久化 JSON 类型

### 配置来源与单写者

App Server 的用户 MCP 与 Skill 设置首次成功读取时导入 SQLite。导入内容和权威切换标记是同一行、同一事务；并发导入只采纳一个成功版本。源文件损坏或数据库写入失败时不创建行、不回退到空配置，保留原文件供修复后重试。导入成功后，更新只写 SQLite，旧文件不删除、不写回，也不再作为运行配置读取。工作区 `.mcp.json` 保持文件权威及文件内容 CAS；不从合成目录反写用户或工作区配置。独立宿主可显式选择原文件后端。

来源写入成功才发送不含配置正文的修订通知；通知不代表 loader 已采用配置或旧代工作已结算。当前变更仅完成来源持久化，loader 配置发布及 MCP 实例管理另行接入。已采用的模型请求保留原来的工具版本及连接代际。

### 目录与内容

Managed `providers.record` is `{kind:"managed", account, models, modelConfigOverrides, disabledModelIds, version}`. Each model is `{entry, protocol, unavailableReason, zdr}`; the plugin returns the normalized `ModelEntry` and public host protocol. Only non-secret account identity is stored here. Refresh and access tokens remain in the separately supplied plugin's account store. Managed and static variants share the existing `providers` table; the v10 table and column layout is unchanged, with the format marker advanced to v11. Provider catalog replacement and user overrides are owned by the App Server, as described in the [Provider contract](../../../crates/dim-app-server/README.md).

静态目录的 record 为 `{kind, displayName, driverKind, auth, credentialAuthority, baseUrl, network, headers, credential, models, remoteModels, endpointTypes, modelConfigOverrides, disabledModelIds, enabled, definitionFingerprint, version}`。`kind` 为 `builtin` 或 `custom`；models 和覆盖均为完整 `ModelEntry`；`definitionFingerprint` 是 bundled 定义的 SHA-256，自定义记录为 null。`credentialAuthority` 为 `static`、`openai_codex` 或 `grok`，缺省仅解释为静态来源；后两者只允许对应内置 Provider 的固定非秘密绑定，credential 必须为空。`credential` 可保存静态凭据或不透明秘密引用，但不写回引用解析结果；`network` 保存逐连接代理与超时，管理面不返回代理 URL。`remoteModels` 标记已取得的独立模型快照，避免 bundled 升级覆盖；`endpointTypes` 保存显式协议映射，派生路由不落盘。凭据和 header 值只由 resolver 使用，管理面仅返回非秘密摘要。写入及读取边界校验模型、地址和请求头，外层 SQL 版本须与 record.version 一致；seed、编辑和删除规则见[Provider合同](../../../crates/dim-app-server/README.md)。

`agents.profile` 是 `AgentProfile`：`{name, description, systemPrompt, tools, model?}`；`model` 若存在为 `{providerId, modelId}`。`agent_sessions.config` 是 `SessionConfig`：`{permissionPolicy, cwd, model, systemPrompt, tools, collaborationRole}`；其中 `model` 是 `ModelSelection {providerId, modelId, config?, metadata?}`，创建时固定完整模型参数，省略参数的身份记录仍可读取。`permissionPolicy` 为 `{preset, allowedTools, disallowedTools}`，`collaborationRole` 为 `leader` 或 `member`。`timeline_branches.info` 是 `TimelineInfo`：`main` 为 `{id:"main"}`，子分支为 `{id, parent, branch_point, create_anchor}`；后两项是父分支的本地序号。JSON 字段名按各类型的 serde 定义，不能由 SQL 列名的拼写推断。

`context/configured` 的 `ModelConfiguration` 分别固定模型与工具部分：模型使用上述参数，`tools` 为已采用声明，可选 `toolSources` 为 `{key,version}[]` 的来源版本；没有来源时省略。模型更新保留已采用的工具部分，工具采用保留模型部分，完整合同见[App Server合同](../../../crates/dim-app-server/README.md)。

`timeline_content.body` 的 `message` 是 `{role,content}`，`role` 为 `user`/`assistant`/`tool_result`，`content` 为文本或 `MessagePart[]`；`tools` 是 `ToolDefinition[]`，每项为 `{name,description,inputSchema,source,risk?,metadata?,fileChanges?}`，嵌套的 schema/metadata/fileChanges 保持现有类型定义；`system` 是文本或 `SystemPromptPart[]`，当前部件为 `{type:"text",text,cache_control?}`。三类正文均直接按对应运行时类型的 JSON 序列化，不增加存储专用字段。

### 事件封装

`timeline_events.body` 的完整外层联合为：

```text
Row =
  { encoding: "event", record: WireEvent }
  | { encoding: "output", turn_id: string, request_id: string,
      started_at: integer, records: WireEvent[] }
WireEvent = { event: AgentEvent, refs: References | null,
              trace: WireTrace | null, batch_end: integer }
References = { messages: string[], tools: string | null, system: string | null }
WireTrace = { event_id: string, original: WireDraft, steps: WireStep[] }
WireDraft = { event: EventDraft, refs: References | null }
WireStep = { handler: string, result: WireDraft | null }
AgentEvent = { id: string, sequence: integer, create_at: integer,
               tags?: string[], type: string, name: string, data: object }
EventDraft = { tags?: string[], type: string, name: string, data: object }
```

`AgentEvent` 与 `EventDraft` 的 `{type,name,data}` 是现有 `EventKind` 标记联合，`data` 由该变体的类型定义（如 `ModelCallData`、`ContextCompactionData`）校验；存储没有第二套业务事件定义。`ModelRequest` 正文为 `{providerId,modelId,messages,tools?,system?,config?,metadata?}`，末尾四项是可选字段，缺省时省略，不能写 `null`。`event` 行有且仅有一个 `record`，其 `sequence = last_sequence = timeline_events.sequence`；`output` 行的 `records` 非空、同一 `kind`/turn/request，逐条保留 ID、sequence、`create_at`、tags、delta、refs、trace 和各自 `batch_end`。输出行只在 `step/text` 或 `step/thinking` 出现，`turn_id`/`request_id` 提到行级并从各 `record.event.data` 移除，读取时补回；`started_at` 是首事件的 `create_at`。合并不改变逻辑事件或原子批次，`last_sequence` 必须等于最后一条的序号。

写入时只有前一物理行与当前增量同 `kind`、同 `turn_id`/`request_id`，前一行编码长度小于 4096 字节，且当前事件的 `create_at - started_at < 1000` 毫秒，才向该行追加；否则新建 `output` 行。阈值检查发生在追加前，故追加后的行可以超过 4096 字节。读取现有输出行不依赖这些写入侧阈值，只校验行内事件与区间。

### 请求引用与 before 轨迹

仅 `context/prepare` 与 `model/call` 的 `data.request` 可带 `refs`：原 `ModelRequest.messages` 各项按顺序写成 `kind=message` 的内容行，`tools`/`system` 若存在分别写成 `kind=tools`/`system`；存储中的 `request.messages` 为 `[]`，`tools` 和 `system` 字段移除，其余请求字段保持原样。`refs.messages` 即使为空也存在，缺席的 `tools`/`system` 在 `References` 中为 `null`。其他事件及非请求 before 草稿的 `refs` 为 `null`。`WireTrace.original` 和每个非空的 `WireStep.result` 也按同一请求规则编码；没有 before 轨迹时 `trace` 为 `null`。这些 `null` 属于存储封装，事件类型自己声明为可选的字段仍按原 serde 规则省略。

### 可丢弃恢复快照

`context/compaction` 的 completed 事实仅保存 source through、cursor、summary、baseline 和 diagnostics，不含 snapshot。快照在事实提交后由独立异步操作写入 `timeline_snapshots`，保存失败不影响事实提交。`RecoverySnapshot={sequence,event_id,interpreter,state}`；默认解释器合同为 `dim/context-projection/v2`，其 state 是 `{payload,sha256}` 的 JSON 字符串，payload 是下列完整 `ProjectionCheckpoint` 的原始 JSON 字符串。SQLite 仍将 state 作为不透明 TEXT 保存，数据库格式保持 v13。

摘要算法为 SHA-256：先写入 UTF-8 `dim/recovery-snapshot/v1` 和一个 NUL，再写入 sequence 的 8 字节大端编码，随后依次写入 event_id、interpreter、payload；每个字符串先写入 UTF-8 字节长度的 8 字节大端编码，再写入原始字节。sha256 保存小写十六进制。解释器在解码 payload 前校验摘要，覆盖 resume、全部状态及锚点身份。旧 v1 无校验快照直接绕过，不通过补算摘要升级；v2 不接受裸检查点或缺失摘要。历史事实及摘要文本不受影响。

`HistoryRenderer` 同时拥有 `encode_checkpoint` 和 `decode_checkpoint`，自定义格式必须成对提供并管理自己的 interpreter 标识。完整性校验检测编码后发生的损坏，不是认证或生产者语义证明：可信 builder 必须生成与完整事实重放等价的状态，并通过差分测试验证。已知 builder 语义缺陷需要修复并更换解释器版本，不能给错误状态重算摘要。有效快照仍只读取恢复位置以后的事实，不在每次读取时重复完整重放。

候选查询只选择相同 interpreter 且 sequence 不大于固定 through 的最新快照，并连接事件锚点验证 ID、序号和 completed 种类；孤立或元数据不匹配的快照被忽略。Session 恢复不校验可选 state；缺失、不兼容、损坏或越界快照回退事实重放，summary 仍保留。运行时在裁剪前校验解码后的 resume 属于固定祖先链和 source through。`Timeline.context` 默认完整重放；加速显式调用 `context_with_checkpoint`，默认能力使用 `history::read_context`。事实或必需引用损坏仍失败并保留原字节。回滚先删除 snapshots 再删除 events。

```text
ProjectionCheckpoint = {
  resume: { branch_id: string, sequence: integer },
  state: {
    compacted_through: string,
    prompts: { entries: PromptEntry[], revision: PromptReference | null,
      system_revision: PromptReference | null, system: string,
      turns: string[], tasks: PromptScope[] },
    delivered: ContextBaselineEntry[],
    inputs: [string, InputMessageData][],
    configuration: ContextConfiguredData | null,
    output: MessagePart[], open_tools: string[], output_open: boolean
  }
}
```

上述检查点中的叶子类型仍是事件类型的原 JSON 形状，不另做存储转换：`PromptReference={timeline,revision}`；`PromptScope={kind:"branch",timeline}` 或 `{kind:"turn"|"task",timeline,id}`；`PromptEntry={source,version,body,scope,delivery,order}`，其中 `delivery` 为 `system`/`context`/`state`。

`ContextBaselineEntry={key,sourceEvent,message}`；`InputMessageData={submission_id?,user?,blocks?,mode?}`，`user.content` 是文本/图片块列表。`blocks` 项只有原文一种：`{type:"raw",text,key?,retire?}`，`text` 是能力写入的最终文本，`key` 表示仍然有效的内容，`retire:true` 结束该 key；语义见[架构约定](../../architecture/README.md)。

`ContextConfiguredData={timeline,revision,previous?,prompt?,settings,system?}`，`settings={providerId,modelId,tools?,toolSources?,config?,metadata?}`；`toolSources` 是 `{key,version}[]`，为空时省略。`Message` 是 `{role,content}`，`content` 是文本或 `MessagePart[]`；`MessagePart` 保持 `text`、`image`、`tool_use`、`tool_result`、`thinking` 的标记联合；text / thinking / tool_use 可保存闭合 `providerMetadata`，原生签名、Responses item 身份及 summary 分组见 [模型宿主](../../../crates/dim-agent-aimux/README.md)。内部 `step/model-content` 保存成功输出的完整有序 parts；增量事件仍服务流式显示。

### 读取校验

读取时先解析 `Row` 和运行时事件，再用 `References` 按 `(session_id, hash)` 查内容、复核 hash 与 kind，并重建 `Arc<ModelRequest>`；随后校验序号、行 kind、批次、before 的 `event_id`、分支关系和所有 completed 事实的 through 边界。可选 snapshot state 不参与 Session 恢复校验。坏行保留原字节并归属该 Session；不得通过默认值把损坏请求还原为空请求。

## 格式版本

`agent_store_format.version` 同时标识 SQL 结构、持久化 JSON 和内容寻址规则。已发布格式发生不兼容变化时，必须定义新版本及读取或转换路径。保持版本的条件是旧读取器能正确解释新值、旧写入器不会重写丢失新值，且存储不变量不变。纯运行时重构不改变格式版本。

首版定稿前的开发库兼容范围由上层持久化设计明确，不预设下一版本或迁移字段。

## 索引与实际查询

| 版本与索引 | 精确列序/条件 | 服务的查询 |
| --- | --- | --- |
| v1 主键 `agents`、`agent_sessions` | 各自 ID | 单 Agent/Session 查找；Agent 全目录目前仍按 `is_default DESC, agent_id` 扫描排序 |
| v1 主键 `session_owners` | `(session_id)` | Session 到 Agent/协议版本；v1 全 owner 目录按 `session_id` 扫描 |
| v1 主键 `timeline_branches` | `(session_id, branch_id)` | 分支 head、整 Session 分支目录 |
| v1 主键 `timeline_events` | `(session_id, branch_id, sequence)` | 追加前读取最后一行、单分支顺序恢复、从起点读取历史 |
| v1 主键 `timeline_content` | `(session_id, hash)` | 请求引用的定点读取和 Session 内去重 |
| v1 唯一 `agents_default_unique` | `(is_default) WHERE is_default = 1` | 至多一个默认 Agent；不保证其存在 |
| v12 主键 `timeline_snapshots` | `(session_id, branch_id, interpreter, sequence)` | 兼容 interpreter，`sequence<=through ORDER BY sequence DESC LIMIT 1`；连接事件锚点 |

## 分页查询

以下索引已与表结构一起创建，供目录过滤、事件分页和跨页 ToolCall 读取使用。事件 ID 索引非唯一，恢复时逐事件查重并核对索引值。

| 索引 | 精确列序/条件 | 查询 |
| --- | --- | --- |
| `session_owners_by_agent` | `(agent_id, protocol_version, session_id)` | 按 Agent 和协议过滤目录 |
| `timeline_events_by_end` | `(session_id, branch_id, last_sequence, sequence)` | 正向页 `last_sequence >= after+1`；反向页同时处理跨界行 |
| `timeline_events_by_id` | `(session_id, event_id) WHERE event_id IS NOT NULL`，非唯一 | 按事件 ID 查找跨页 ToolCall |
| `timeline_tool_use_starts` | `(session_id, json_extract(body, '$.record.event.data.request_id'), json_extract(body, '$.record.event.data.call_id')) WHERE branch_id = 'main' AND kind = 'step/tool-use-start'` | 按请求与调用 ID 查找跨页 `step/tool-use-start`，为不带 `call_event_id` 的结果取调用名；查询须使用相同表达式与条件 |

正向页的 `after` 与 `through` 须为批次边界；`limit` 为目标事件数，末批超出时完整返回。页查询以 `last_sequence` 为 seek/排序列。合并行覆盖页起点时完整解码，再按内部 sequence 过滤；反向页须纳入 `sequence < before <= last_sequence` 的跨界行，并停在原子批次边界。正向读取按 256 个逻辑序号分段，恢复校验按 64 条物理行分段，每段结束释放连接锁。单条物理行完整解码，允许超过通常页大小。流程测试用 `EXPLAIN QUERY PLAN` 核对正向读取索引；`timeline_acceptance paged-reads` 测量长历史读/恢复与并发提交延迟。反向历史接口仍为独立协议草案。

当前两个 ID 列的 SQL 可空缺口见表结构。永久关闭字段、协作表和旧记录回填在业务范围确定后另行设计。

## 完整记录示例（v13）

下例是一条 Session `s1`、主分支 `main` 和子分支 `child` 的全部原生表记录，用于核对存储关系，不代表完整的对话执行序列。为便于阅读，`profile`、`config`、`info` 和事件 `body` 这几个实际为 `TEXT` 的列在展示时解析为 JSON；写入时各自是完整 JSON 字符串，类型规定省略的可选字段照常省略。ID/时间是示例值。`timeline_content.body` 保留实际写入的字符串形式，因此下面两个 hash 可以按上文算法复核。

示例刻意保留旧 `dim/context-projection/v1` 快照，验证 v13 数据库无需迁移：默认 v2 解释器绕过这条无校验缓存，仍从事实恢复相同 summary、保留消息和子分支。新快照使用上节的完整性封装。

```json
{
  "agent_store_format": [{"version": 13}],
  "settings_documents": [],
  "providers": [],
  "provider_selections": [],
  "provider_imports": [],
  "agents": [{"agent_id": "a1", "is_default": 1, "profile": {
    "name": "Example", "description": "Example agent", "systemPrompt": "You are helpful.",
    "tools": [], "model": {"providerId": "mock", "modelId": "deterministic"}
  }}],
  "agent_sessions": [{"session_id": "s1", "created_at_ms": 1000, "config": {
    "permissionPolicy": {"preset": "read-only", "allowedTools": [], "disallowedTools": []},
    "cwd": "/work", "model": {"providerId": "mock", "modelId": "deterministic"},
    "systemPrompt": "You are helpful.", "tools": [], "collaborationRole": "leader"
  }}],
  "session_owners": [{"session_id": "s1", "agent_id": "a1", "protocol_version": 3}],
  "timeline_branches": [
    {"session_id": "s1", "branch_id": "main", "info": {"id": "main"}, "sequence": 8},
    {"session_id": "s1", "branch_id": "child", "info": {
      "id": "child", "parent": "main", "branch_point": 7, "create_anchor": 8
    }, "sequence": 2}
  ],
  "timeline_content": [
    {"session_id": "s1", "hash": "59cdf8dcbefa00e8fa27452182332492a68dccb11804c2bd571c53f1750c4e69",
     "kind": "message", "body": "{\"role\":\"user\",\"content\":\"Hi\"}"},
    {"session_id": "s1", "hash": "eed2128b01236e0436f9b7ae565767c078e5e6f1d13dcacd51fa8399564c6288",
     "kind": "system", "body": "\"You are helpful.\""}
  ],
  "timeline_events": [
    {"session_id": "s1", "branch_id": "main", "sequence": 1, "last_sequence": 1,
     "event_id": "e1",
     "kind": "input/message", "body": {"encoding": "event", "record": {
       "event": {"id": "e1", "sequence": 1, "create_at": 1001, "type": "input", "name": "message", "data": {"user": {"content": [{"type": "text", "text": "Hi"}]}}},
       "refs": null, "trace": null, "batch_end": 1
     }}},
    {"session_id": "s1", "branch_id": "main", "sequence": 2, "last_sequence": 2,
     "event_id": "e2",
     "kind": "input/consumed", "body": {"encoding": "event", "record": {
       "event": {"id": "e2", "sequence": 2, "create_at": 1002, "type": "input", "name": "consumed",
                 "data": {"input_id": "e1"}},
       "refs": null, "trace": null, "batch_end": 2
     }}},
    {"session_id": "s1", "branch_id": "main", "sequence": 3, "last_sequence": 3,
     "event_id": "e3",
     "kind": "context/compaction", "body": {"encoding": "event", "record": {
       "event": {"id": "e3", "sequence": 3, "create_at": 1003, "type": "context", "name": "compaction", "data": {
         "turn_id": "t1", "request_id": "r1", "through": 2, "status": "completed",
         "cursor": "e2", "summary": "Hi",
         "diagnostics": {
           "reason": "auto", "candidateMessageCount": 1.0, "inputMessageCount": 1.0,
           "retainedMessageCount": 0.0, "retainedTokenBudget": 128.0, "retainedTokens": 0.0,
           "serializedChars": 2.0, "estimatedInputTokens": 1.0,
           "estimatedCompressedTokens": 1.0, "estimatedPostCompactionTokens": 1.0,
           "summaryInputBudget": 128.0, "preflightInputTokens": 1.0,
           "preflightOutputTokens": 1.0, "compactedToolPayloadCount": 0.0,
           "droppedMessageCount": 0.0, "chunkCount": 1.0, "summaryChars": 2.0,
           "estimatedSummaryTokens": 1.0, "modelCallCount": 1.0, "durationMs": 1.0
         }
       }},
       "refs": null, "trace": null, "batch_end": 3
     }}},
    {"session_id": "s1", "branch_id": "main", "sequence": 4, "last_sequence": 4,
     "event_id": "e4",
     "kind": "model/call", "body": {"encoding": "event", "record": {
       "event": {"id": "e4", "sequence": 4, "create_at": 1004, "type": "model", "name": "call", "data": {
         "turn_id": "t1", "request_id": "r1", "request": {
           "providerId": "mock", "modelId": "deterministic", "messages": []
         }
       }},
       "refs": {"messages": ["59cdf8dcbefa00e8fa27452182332492a68dccb11804c2bd571c53f1750c4e69"],
                "tools": null, "system": "eed2128b01236e0436f9b7ae565767c078e5e6f1d13dcacd51fa8399564c6288"},
       "trace": null, "batch_end": 4
     }}},
    {"session_id": "s1", "branch_id": "main", "sequence": 5, "last_sequence": 6,
     "kind": "step/text", "body": {
       "encoding": "output", "turn_id": "t1", "request_id": "r1", "started_at": 1005,
       "records": [
         {"event": {"id": "e5", "sequence": 5, "create_at": 1005,
           "type": "step", "name": "text", "data": {"delta": "He"}},
          "refs": null, "trace": null, "batch_end": 6},
         {"event": {"id": "e6", "sequence": 6, "create_at": 1006,
           "type": "step", "name": "text", "data": {"delta": "llo"}},
          "refs": null, "trace": null, "batch_end": 6}
       ]
     }},
    {"session_id": "s1", "branch_id": "main", "sequence": 7, "last_sequence": 7,
     "event_id": "e7",
     "kind": "step/model-output-end", "body": {"encoding": "event", "record": {
       "event": {"id": "e7", "sequence": 7, "create_at": 1007, "type": "step", "name": "model-output-end",
                 "data": {"turn_id": "t1", "request_id": "r1", "status": "completed", "finish_reason": "end_turn"}},
       "refs": null, "trace": null, "batch_end": 7
     }}},
    {"session_id": "s1", "branch_id": "main", "sequence": 8, "last_sequence": 8,
     "event_id": "e8",
     "kind": "branch/create", "body": {"encoding": "event", "record": {
       "event": {"id": "e8", "sequence": 8, "create_at": 1008, "type": "branch", "name": "create", "data": {
         "at": {"kind": "sequence", "sequence": 7},
         "source": {"kind": "host", "operation_id": "op1"},
         "branch": "child", "prompt": "Follow up"
       }},
       "refs": null, "trace": null, "batch_end": 8
     }}},
    {"session_id": "s1", "branch_id": "child", "sequence": 1, "last_sequence": 1,
     "event_id": "child-e1",
     "kind": "branch/start", "body": {"encoding": "event", "record": {
       "event": {"id": "child-e1", "sequence": 1, "create_at": 1009, "type": "branch", "name": "start", "data": {
         "parent": "main", "createEvent": 8
       }},
       "refs": null,
       "trace": {"event_id": "child-e1", "original": {
         "event": {"type": "branch", "name": "start", "data": {"parent": "main", "createEvent": 8}},
         "refs": null
       }, "steps": [{"handler": "example", "result": null}]},
       "batch_end": 1
     }}},
    {"session_id": "s1", "branch_id": "child", "sequence": 2, "last_sequence": 2,
     "event_id": "child-e2",
     "kind": "input/message", "body": {"encoding": "event", "record": {
       "event": {"id": "child-e2", "sequence": 2, "create_at": 1010, "type": "input", "name": "message",
                 "data": {"user": {"content": [{"type": "text", "text": "Follow up"}]}}},
       "refs": null, "trace": null, "batch_end": 2
     }}}
  ],
  "timeline_snapshots": [{"session_id": "s1", "branch_id": "main", "sequence": 3,
    "interpreter": "dim/context-projection/v1", "event_id": "e3",
    "state": "{\"resume\":{\"branch_id\":\"main\",\"sequence\":2},\"state\":{\"compacted_through\":\"e2\",\"prompts\":{\"entries\":[],\"revision\":null,\"system_revision\":null,\"system\":\"\",\"turns\":[],\"tasks\":[]},\"delivered\":[],\"inputs\":[[\"e1\",{\"user\":{\"content\":[{\"type\":\"text\",\"text\":\"Hi\"}]}}]],\"configuration\":null,\"output\":[],\"open_tools\":[],\"output_open\":false}}"
  }]
}
```

这十条逻辑事件占九条物理事件行；`main` 的已发布 head 为 8，`child` 为 2。`input/consumed` 把 `e1` 转成 ID 为 `e2` 的用户历史条目，因此压缩游标指向 `e2`；可选快照从第 2 条事件重放，所保存的前缀状态含尚未采纳的 `e1` 输入。`main` 的输出行合并两个增量，读出后各自恢复 `turn_id`/`request_id`，两者的原子批次终点都是 6。子分支通过 `branch_point=7` 共享 `main` 的前缀 1–7，不复制父事件；`create_anchor=8` 指向父分支的 `branch/create`。`model/call` 的两个 hash 在同一 Session 的 `timeline_content` 命中。`timeline_snapshots` 的独立行锚定主分支第 3 条 completed 事实，事件本身没有快照数据；删除该快照不影响事实重放或 summary。回滚 Session 时先删除快照，再删除事件、分支、内容和 owner；Agent 只有在不再被任何 Session 引用时才能删除。
