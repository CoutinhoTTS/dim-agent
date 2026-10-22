# Provider management

Status: **Proposal; not Ready and not implemented**. Extends the existing process
`ProviderPlugin`. The implemented S4 [`auxiliaryModel`](../appserver/auxiliary-model.md)
remains complete and unchanged. Tracking: [#329](https://github.com/arcships/dim/issues/329)
and the [roadmap](../roadmap.md).

本方案让 Desktop 与 CLI 通过同一 App Server 完成 §1 的 Provider 管理流程。账户用量
仅通过可选账号插件查询，静态 API-key Provider 不承担该能力。新增方法和字段均为提案；客户端不能据此调用尚未发布的接口。
当前行为以源码和既有合同为准。设计确认、Provider 实现与客户端验收分别记录。

## 1. Scope and current implementation

功能基线见 [Providers and models](../../feature-list.md#providers-and-models)，包括
创建前探测、连接管理、模型编辑、账号流程、账户用量和模型选择。

下表按 `main` @ `78dd8c5e` 的实现区分已有能力和缺口：

| User flow | Implemented | Proposed completion |
| --- | --- | --- |
| 列表与编辑详情 | `provider.list` 返回静态记录摘要；托管目录另经 `account.providers`；`provider.catalog` 返回有效模型 | 提供统一的管理列表和安全编辑详情，包含来源、账号 authority、操作能力、配置状态和逐条错误 |
| 连接配置 | `provider.save` 可保存认证、URL、headers、代理、超时和启用状态 | 用创建与局部更新入口替代 save，避免客户端重建包含不可回读秘密的整条记录 |
| 断开与删除 | 自定义 Provider 真删除；内置 Provider 的 `delete` 仅禁用 | 明确区分禁用、移除静态凭据、删除自定义记录和账号登出 |
| 创建前获取模型 | `provider.refresh` 要求已保存的 Provider | 用未保存的连接草稿探测，不创建临时 Provider，不写入凭据或目录 |
| 模型管理 | 已有刷新、启停、可见字段编辑和重置；编辑要求模型存在于默认列表 | 新增用户模型、删除仅用户添加的模型；默认模型、用户覆盖和用户新增模型保持不同语义 |
| 模型能力 | 已有 limits、tool calling、vision、reasoning 和媒体能力编辑 | 复用 `ModelEdit`；补齐管理来源/操作说明以及 New API 的显式路由编辑 |
| 模型偏好 | 会话创建和 `session.model.update` 接受 `ModelPreferences`；F3 已交付 | 保存、回读与清除按模型的用户偏好，并接通客户端显式采用流程 |
| 账号 | 原生 Codex/Grok 与可选托管账号已有登录、恢复和登出合同 | 统一呈现状态和可执行操作；登录仍由原账号 owner 执行，客户端按需重读 |
| 账户用量 | 无原 Desktop 账户用量管理接口 | 仅可选账号插件提供一次调用的即时查询；静态 API-key Provider 不提供用量设置或查询 |
| 同步 | 管理调用有回复；`management.progress` 只服务单次调用 | 打开视图和自身写入后重读；并发写入沿用 CAS，与 Settings 一致 |
| 旧配置导入 | 已有显式只读导入和逐条结果；部分非空字段拒绝 | 覆盖持久模型偏好、活动模型等字段；不支持的静态用量设置明确拒绝；保留源字节与重复导入保护 |

证据入口：[Provider methods](../../../crates/dim-app-server/src/provider/management.rs)、
[Provider records and mutations](../../../crates/dim-app-server/src/provider.rs)、
[ModelEdit](../../../crates/dim-app-server/src/model_catalog.rs)、
[legacy import](../../../crates/dim-app-server/src/provider/legacy.rs)、
[F3 contract](../model-configuration.md)。

范围不包含重做页面设计、远程设备管理授权、媒体任务执行器或共享凭据存储。
媒体能力声明由 Provider 保存；执行可用性和默认媒体模型归 [Modality](Modality.md)。
辅助模型、子代理与翻译等用途的选择归消费插件的业务配置，不增加 Provider purpose 表。

## 2. Ownership and exports

Provider 是现有进程插件，不增加 Session/branch Capability 或 Management 接线插件。
业务类型、校验、目录投影、写入和任务结算由 Provider owner 实现；Rpc 和 Transport
只负责既有通用调用入口，不能加入 Provider 分支。已有实现位于 App Server 的 Provider
模块，不将新业务逻辑放进 `connection.rs`、`protocol.rs` 或客户端网关。

| Owner | State and responsibility |
| --- | --- |
| Provider | Provider 记录、连接配置、用户模型/覆盖、显式禁用集合、默认选择、按模型偏好；有效模型与管理投影；本代任务 |
| Account owners | 登录尝试、账号状态、token、刷新策略、凭据租约；可选账号插件还可通过自己的管理方法提供即时用量查询 |
| SqliteStorage | 原子事务、格式门禁及受支持格式迁移；不解释 Provider 的业务字段 |
| Rpc/Connection | 本地准入、namespace 路由、既有方法调用与连接关闭 |
| Session/branch configuration owners | 通过已有配置命令采用模型与参数，保存其必需 Timeline 事实 |
| Clients | 草稿、搜索、模板选择与展示；提交用户意图，消费最终投影；不保存第二份 Provider 配置 |

最终对外面如下；`ProviderService` 的 Rust 方法不是额外的跨语言公共 API：

| Surface | Consumers | Contract |
| --- | --- | --- |
| `managementMethods`, process, key `provider` | 本地 v3 Desktop/CLI；受信任进程调用者 | §3 的可序列化方法与错误；沿用现有 `describe/call` 绑定 |
| `account`、`account.codex`、`account.grok` 管理 namespace | 客户端账号交互 | 保持各自 owner；Provider 管理视图只返回所属 namespace、支持的登录方式与状态 |
| `ProviderConnectionResolver`、`ModelSelectionHost` | 现有原生模型调用与选择入口 | 保持现有类型合同；解析当前连接，不向终端 RPC 返回凭据 |
| `auxiliaryModel.complete`，process，无实例键 | 无法持有凭据的消费插件 | 保持 S4 的具名数据合同；调用者显式提供模型，不增加用途策略 |

内部服务只在真实跨语言消费边界提供具名绑定；不把所有内部函数逐一导出。
可选账号插件保持[公开宿主边界](../../architecture/development.md#managed-provider-boundary)，
具体账号体系、realm 和 token 流程不进入公开仓库。

## 3. Proposed management contract

### 3.1 Read models

沿用现有 `ProviderModel`、`ModelEntry`、`ModelEdit`、`ModelPreferences`、`StaticAuth` 和
`ModelNetwork` 的单一定义。管理视图只增加必要的业务说明，不从可见字段重建另一种模型配置。

| Method | Request | Reply and meaning |
| --- | --- | --- |
| `provider.management` (new) | `{providerId?}` | `{drivers, providers: ManagementProvider[], errors: ProviderRecordError[]}`；合并静态与已装载账号 owner 的条目，并提供安全编辑字段；省略 ID 返回全部，指定 ID 只读该项；不按“已登录”过滤 |
| `provider.selection` (existing) | `{cwd?}` | 保持现有 `{selection}`，全局与工作区选择分开；其 version 不能用作 Provider 记录的 version |
| `provider.catalog` (existing, extended) | `{providerId}` | 保持 `{catalog}` 及有效 `ProviderModel` 的单一定义；附模型来源及可用管理操作，仍供选择和执行消费者读取 |

`drivers` 是当前宿主允许自定义创建的 `{driverKind, authTypes, remoteModelList}` 列表，来源为
同一 driver 实现；不提供当前无法执行的选项，不增加动态表单 schema 或第二份 driver 注册表。

`ManagementProvider` 包含现有摘要、`version`、来源和 credential authority，增加
原 owner 的公开状态、诊断、`actions`、安全的 `connection` 与按 modelId 保存的
`ModelPreferences`。编辑字段由同一管理读取提供，模型列表复用 provider.catalog。
其中 `version:null` 仅表示账号 owner 已公布该 Provider、但本地目录记录尚未建立，不能用 0
冒充记录版本；此时只开放相应登录/刷新动作。指定不存在的 ID 返回 provider_not_found；
模型目录读取失败返回明确错误，不能映射成空目录。
`actions` 只列出当前可用的操作名称；账号操作沿用其 owner 的公开结果。模型操作区分
toggle、edit、reset、delete，不增加逐操作的原因代码表。公开诊断说明条目错误；服务端在调用时
仍检查当前业务状态，action 不是授权凭证或写入承诺。

模型来源区分默认目录项、默认项上的用户覆盖和仅用户新增项。模板从已读取的有效目录派生，
以 `{providerId, modelId}` 标识来源；复制的是完整公开 descriptor，不保存模板引用，不另打包目录。
用户新增/覆盖的来源需要持久保留；刷新移除默认项不能把原覆盖误判成用户新增项。来源只说明
同一 ModelEntry 的管理语义，不复制一套模型字段；具体记录形状属于 §6 的格式决定。

`connection` 返回非秘密配置与秘密配置状态。凭据、secretRef、header 值和代理认证内容不回传；
headers 只公开名称，代理只公开不含认证的地址及认证已配置标记。该地址只用于显示，不能自动写回。
“已保存”是独立状态，不能用假 token 或占位字符串填入待保存字段。

列表允许局部失败：一个坏记录或账号错误进入 `errors`，健康 Provider 仍可读、可用。
详情返回该条明确错误，不能用空目录或未配置状态掩盖错误。聚合视图不承诺所有账号的跨 owner
原子快照；`version` 只保护所属 Provider 记录，账号状态直接使用其 owner 的公开结果。

### 3.2 Connection and model writes

| Method | Request | Result / semantics |
| --- | --- | --- |
| `provider.create` (new) | `{displayName, connection, models: ModelEntry[], enabled}` | 管理详情；服务端分配不可复用的自定义 providerId，不从显示名称生成 identity |
| `provider.update` (new) | `{providerId, expectedVersion, patch}` | 更新后的管理详情；patch 仅含用户改变的配置/启用字段；省略保留，写入前校验并在一个事务中提交 |
| `provider.delete` (existing) | `{providerId, expectedVersion}` | 保持旧合同；新客户端仅对允许 delete 的自定义项调用，删除及选择清理同事务；内置项的旧调用仍只禁用 |
| `provider.probe` (new) | `{providerId?, expectedVersion?, connection}` | `{models: ProviderModel[]}`；§3.3 的只读网络探测 |
| `provider.models.create` (new) | `{providerId, expectedVersion, model: ModelEntry}` | 更新后的 catalog；用户新增项写入现有覆盖集合，重复 ID 明确拒绝 |
| `provider.models.delete` (new) | `{providerId, expectedVersion, modelId}` | 更新后的 catalog；只删除用户新增项，默认项使用禁用或 reset；相关偏好、路由和失效选择在同事务处理 |
| `provider.models.edit` (existing, extended) | 现有请求；edit 增加 `endpoint` 操作 | 保留现有字段 patch 与隐藏元数据；支持 New API 显式路由 override 的 set/clear |
| `provider.models.reset` / `set_enabled` (existing) | 保持现有请求 | reset 仅针对存在默认配置的项；启停覆盖完整有效目录，包含用户新增模型 |
| `provider.refresh` (existing) | 保持现有请求 | 更新远端默认目录，不覆盖用户模型、用户配置、偏好或显式禁用；保留当前 admitted-snapshot 冲突检查 |

`provider.create` 与 `provider.update` 替代 `provider.save`，完成客户端迁移后移除旧入口，
不保留绕过局部写入合同的第二条公共写路径。当前 CLI 的 Provider 能力发现、连接向导及启停
调用（`crates/dim-cli/src/session_host.rs`）和相关公开流程需在同一实施范围迁移；已有调用者按
方法集合检查版本，新客户端不能回退到 save。

Disconnect 是一次 `provider.update`：同一 patch 原子提交 credential clear 与 enabled false，
保留模型及其他配置；删除自定义记录仍用 delete，账号登出仍用账号 owner 的既有方法。
内置定义的 identity、driver 和认证定义继续只读；新增模型是用户覆盖，不改内置 catalog 文件。
`default-catalog.json` 继续作为 Provider 装载的内置数据，客户端不读取资源文件或数据库。

patch 使用业务字段的明确操作，不引入通用 JSON Patch 解释器：

- 普通值省略即保留；需要恢复默认的字段使用 `{action:"clear"}`，设置使用 `{action:"set", value}`。
- 凭据沿用现有 keep/clear/set/set_aws/set_ref 操作及 authority 检查；解析的秘密只存在于请求内。
- headers 采用逐个名称的 set/remove；省略的名称保留。大小写识别在输入边界一次完成。
- network 按 proxy、timeoutMs 分别更新；仅改超时不能覆盖旧代理；替换代理 URL 明确替换其认证。
- 不支持的字段拒绝，不能静默忽略；取消、校验失败和版本冲突不提交部分 patch。

New API 的 `endpoint` 为 `{action:"set", value}` 或 `{action:"clear"}`，值使用现有可执行
chat endpoint 类型。用户 override 优先于发现结果；clear 回到发现/目录路由，不删除原始 metadata。
未知上游 endpoint 字符串保留作描述，不自动当成可执行路由。有效模型、可用性判断和实际请求
必须读取同一结果；新增模型不能只出现在页面上。

### 3.3 Probe, preferences and account usage

探测新连接时 `providerId` 和 `expectedVersion` 同时省略，connection 提供完整必需字段；
探测已有连接的编辑草稿时两者同时存在，connection 是 §3.2 的 patch，Provider 在内存中合并
同版本配置并按原 authority 获取凭据。不得先保存或临时创建记录以复用 refresh。
返回值不包含凭据、HTTP header 或原始错误响应。客户端只采用仍匹配当前草稿的请求结果。
不支持远程列模型的 driver 返回 `provider_unsupported`，仍允许手动配置模型。

| Method | Request | Result / semantics |
| --- | --- | --- |
| `provider.preferences.update` (new) | `{providerId, expectedVersion, modelId, patch}` | 更新后的管理详情；reasoningEffort、serviceTier 分别 set/clear；校验目录声明，清除表示采用模型默认 |

保存模型偏好不改变已经采用的 Session 配置。客户端读取 Provider 保存的标准
`ModelPreferences` 后，按用户操作显式传给现有 `session.create` 或 `session.model.update`；
不解析 provider-specific request options。本方案以 Provider 持久偏好替代
[F3](../model-configuration.md) 的“不新增持久偏好”范围限制；服务端不自动应用它们。
Session 方法的采用语义不变：
显式目标省略偏好仍从目录默认开始，无目标更新只改变当前分支提供的参数，不偷偷读取持久偏好。
自动标题、子代理等消费方仍自行决定其模型及参数，不自动继承聊天偏好。

账户用量仅由可选账号插件通过自己的管理方法查询：一次调用完成实际查询并返回结果或
安全错误。Provider 不为静态 API-key 条目提供用量设置、探测、CAS 或查询方法；不持有用量
快照缓存和加载/陈旧状态，也不增加账号代次复查。客户端只展示实际查询结果，缺失的指标不
补零，模型调用 token usage 仍走既有事件。

客户端从已装载账号插件的实际方法集合发现查询入口；没有插件或查询方法就不展示该动作，
不影响模型调用及现有登录流程。供应方请求、凭据和错误脱敏都留在私有账号插件。公开托管
host API 不增加用量 capability、账号事件或其他新表面；无需修改 rutis。

### 3.4 Errors, cancellation and compatibility

新管理方法的业务错误使用现有 JSON-RPC `{code, message, data:{appError}}`，增加稳定的
`provider_not_found`、`provider_conflict`、`provider_invalid_config`、`provider_unsupported`、
`provider_auth_required`、`provider_auth_failed`、`provider_transport_failed`、
`provider_cancelled` 与 `provider_failed` 分类。Provider 在边界映射类型错误，不解析内部字符串
决定分类；公开 message 为安全诊断，不拼接 token、代理认证、header 值或原始响应正文。
沿用 Rpc 的 `service_reloading` / `service_unavailable`，不在 Provider 内模拟 owner 就绪。

除由 create/update 替代的 save 外，保留既有方法的请求、回复及错误合同；
新方法通过 `hello.features` 的实际方法集合发现。
修改旧 DTO 的可选字段必须写清能力协商，不能仅因为连接是 v3 就发送扩展字段：
`provider.management` 与本方案的 endpoint 编辑、目录管理字段作为同一可用接口单元发布。
旧客户端不被要求理解新管理回复；新客户端缺少方法时显示版本不满足，不能回退到直接改库。

调用沿用 owner 的 admission、任务追踪和 drain。客户端断开只丢失回复，不撤销已提交写入；
不确定结果先重读，不盲重放。网络任务使用 Provider 的取消 token；释放网络/凭据资源后结算。
业务配置改变不 reload loader 行。保存沿用 expectedVersion CAS；刷新保留既有 admitted
配置冲突检查。客户端不采用过期草稿或页面的探测回复，不新增跨代次校验机制。

## 4. App Server changes

| Area | Required change | Boundary |
| --- | --- | --- |
| Provider owner | §3 的类型化管理读写、模型生命周期与持久偏好 | 复用现有存储、driver 和有效目录；不另造注册表 |
| Rpc/Connection | 通过既有 managementMethods 路由实际方法集合 | 不增加推送通知或连接通道能力，不解释 Provider 数据 |
| Optional account plugin | 通过插件自己的管理方法提供一次调用的用量查询 | 不扩充公开 managed host API，凭据留在原 owner |
| Storage | 在 v18 的 Provider JSON 中增加可选字段 | SQL schema 和格式号不变；接受旧二进制重写时丢失未知字段 |
| Session entry | 复用现有 create/model.update | 客户端显式传递偏好；服务端不自动读取它们 |

管理视图在打开时和自身写入后重读；并发写入由现有 expectedVersion CAS 检出，处理方式与
[Settings](../appserver/settings-service.md) 一致。其他客户端的修改在下次读取时可见；不承诺
实时同步，不增加 Provider 推送通知、账号状态事件、订阅句柄、溢出关闭规则或通知重放。
用户保留编辑草稿时，冲突提示其重读并重新确认，不用新版本号自动覆盖。

### Requirements raised to mechanism owners

None. 本方案通过现有管理调用、重读和 CAS 完成流程，不要求 core、Rpc、rutis、装配、
生命周期或公开托管账号合同增加能力。客户端变更通知如有需求，需与 Settings 一起另行决定，
不是本方案的 Ready 前置。

## 5. Client integration flow

```text
Desktop / CLI                Local App Server               Provider / account owner
    | hello v3, inspect features     |                                |
    | open view: management -------->|------------------------------->|
    | catalog / selection ---------->|<----- safe typed results ------|
    | edit local draft               |                                |
    | create / probe / CAS update -->|------------------------------->|
    |                                |             validate + commit  |
    |<---------- result -------------|<-------------------------------|
    | reread after own write ------->|------------------------------->|
    | create/update Session with explicit model + preferences          |
    | reopen / reconnect: reread                                     |
```

1. 通过同一 product home 的本地 socket / named pipe 协商 v3；配对设备不因此获得管理权限。
2. 打开视图时检查实际方法集合，读取管理条目、选择和所选目录；自身写入后重读相关结果。
3. 未配置项用草稿 probe 或手动添加模型；创建采用返回 identity/version。修改已有项只提交
   changed fields，秘密默认 keep；取消草稿不写入。
4. CAS 冲突时重读并保留草稿，由用户重新确认；单条读取错误不清空整个列表，过期页面回复
   不覆盖当前选择或草稿。其他客户端的改动在下一次读取时反映。
5. 模型新增、编辑、启停、刷新分别执行。搜索只过滤展示，模板复制完整公开 descriptor；
   媒体能力声明不等于执行支持。
6. 登录调用所属账号 namespace 的现有方法；Disconnect 只提交清除凭据并禁用的 update，
   不等于账号 logout 或删除。可选账号用量通过插件自己的单次查询读取，不保存静态用量设置。
7. provider.select 保存默认选择；改变当前 Session 则另行显式提交 session.model.update，
   两次操作分别报告结果。客户端读取持久 ModelPreferences 并显式传入；服务端不替它采用。
8. 断线或重连后重读。不确定的写结果先读取，不能盲重发；不依赖通知或 replay。

界面保持原有组件与布局，下面仅规定必须区分的交互状态，不是替换产品视觉设计：

```text
Providers                         Provider detail
[ Search... ] [+ Add] [Refresh]    Name / source / connection status
  Builtin A         Connected     [Edit connection] [Disable] [Disconnect]
  Custom B          Disabled      Credential: Stored  [Keep | Replace | Clear]
  Account C         Signed out    Models [Search...] [Add] [Refresh]
  Broken D          Error         [ ] All models
                                  Model      Enabled   [Edit] [Reset/Delete]
                                  Account usage: query result [Query]
                                  Unsaved draft: [Cancel] [Save]
                                  Conflict: reload current / retain draft
```

CLI 提供等价功能，不必复制 Desktop 布局。两端均不能根据硬编码 Provider 名称推测操作能力，
不能通过“暂存真实 Provider 再删除”模拟 probe，也不能把未保存值写入客户端持久配置以绕过协议。

## 6. Persistence, migration and settlement

默认目录、用户覆盖/新增、禁用集合与连接保持唯一记录；按模型偏好、模型来源和显式路由
作为 Provider JSON 的可选字段增加。默认选择继续使用既有 scope/version；用途配置归消费
owner。模板、搜索、临时探测、账号用量结果、网络 client 与已解析凭据不持久化。

**格式决定：保持 v18。** 不改 SQL schema、不新增存储格式号。Provider JSON 的新增字段是
可选增量，缺少时分别表示无保存偏好、无显式路由和既有目录语义；读取边界按这些业务含义
解析。与 C2 的 interactive 字段一样，接受旧二进制忽略新增字段、重写记录时丢失它们的后果。
这是明确接受的降级数据损失，不增加存储 owner 确认或格式升级前置。实现需以一次性数据库
验证新旧记录及重写行为；未知原生数据库格式仍按现有门禁拒绝并保留字节。

旧 TypeScript 配置沿现有显式只读导入边界迁移，逐条报告 imported/already_imported/rejected：

- 连接、用户模型/覆盖、显式禁用、模型偏好和 global/workspace 选择按逐字段映射处理；
  activeModelId 仅是最后选择提示，不能覆盖已有显式 scoped selection。
- 静态 Provider 的旧用量设置不迁移到新 Provider 字段；遇到这些非空设置明确拒绝并保留源，
  不丢字段后宣称成功。账号插件自行管理其配置；OAuth grants 不经通用导入复制。
- retryConfig/capabilityDefaults 等其他非空字段要有等价映射或明确拒绝，实施前冻结清单。
  已有目标不自动覆盖；失败来源和 import marker 保留，重复导入不复活删除的条目。

create 分配新的自定义 identity；移除 caller-chosen identity 的 save 后，不允许公共写入口
重建同一已删除自定义 identity。沿用记录现有 version CAS，不增加跨删除的版本历史或时钟。
现有 refresh 的 admitted 配置冲突检查保留；本方案不为用量查询增加配置/账号代次检查。

Provider 在 apply 中立即登记资源清理。关闭准入、取消网络 I/O、释放凭据租约并结算已接受
调用后移除服务；不新增观察任务。业务配置更新不 reload，不修改已采用的分支配置。

## 7. Acceptance and delivery

| Gate | Required evidence |
| --- | --- |
| Design Ready | 管理读写、客户端显式偏好采用、v18 可选字段及逐字段导入映射确认；不以文档宣称实现 |
| Safe configuration | 改超时保留代理认证，改一个 header 保留其他 header；keep/set/clear 与 secretRef 流程；日志、错误及 RPC 中无秘密 |
| Model lifecycle | 内置 Provider 新增用户模型并实际调用；默认项 edit/reset、用户项 delete；刷新/重启保留覆盖与禁用；New API 路由在真实请求生效 |
| Probe | 未保存配置可获取模型；取消/失败无 Provider、凭据、选择或目录写入；过期草稿回复不被采用 |
| Preferences | 保存、清除、重启回读；显式传参的新 Session/切换采用正确参数；F3 无目标更新保持原行为；用途模型不被串改 |
| Accounts and usage | 既有账号流程保持；仅装载可选账号插件且公开查询方法时提供用量操作，单次查询返回实际结果或安全错误，无 Provider 用量配置/缓存 |
| Multiple clients | 打开及自身写后重读、两个客户端 CAS 冲突、保留草稿；断线/服务重载后重读，不期待实时推送 |
| Migration | 一次性数据库覆盖完整有效旧配置、冲突、部分拒绝、重复导入、删除后重试、未知格式保留；原始用户数据库不用于测试 |
| End to end | 当前源码构建的 App Server 与实际 Desktop/CLI：配置 → 模型 → 选择 → 调用 → 修改 → 再调用 → 重启 → 断开/删除 |

复用现有 provider、transport、managed-account 和 CLI 流程测试，必要时扩充这些公开流程；
不为 helper、内部调用顺序或 mock 配置新增独立测试。所有开发与验证使用隔离 home。
文档 PR 只检查引用和一致性，不运行 Rust 测试；实现 PR 按受影响合同和仓库检查要求验证。
客户端是否完成以实际流程证据为准，后端单测或 feature 名称不能代替用户流程验收。

## 8. Rule check

| Rule | Application |
| --- | --- |
| §4 rule 1 | Provider 进程状态通过其类型化写操作修改；跨 owner 的 Session 配置沿既有命令和采用事实，不直接改分支状态 |
| §4 rule 2 | 不撤回已交付工作；已提交写入不会因调用方断线撤销；模型采用继续使用现有配置交接 |
| §4 rule 3 | 不新增 before 回调；入口校验 shape/权限，Provider 校验业务，网络不占提交锁 |
| §4 rule 4 | Rpc 只识别既有 namespace 和调用，不解释 Provider；Timeline/Dispatcher 不增加业务字段或分支 |
| §4 rule 5 | 关联的模型/偏好/路由/选择清理同事务；过程级变更不进入 Timeline；现有 Session 原子采用不变 |
| §4 rule 6 | 管理读取和配置不进入模型上下文；实际 Session 变化只经既有采用路径 |
| §4 rule 7 | 账号经公开服务；不读 token 文件、不跨写内部状态；有效目录由 Producer 保证 |
| §4 rule 8 | §4 无机制需求；复用既有管理调用、CAS 和 v18 JSON 增量，不修改 rutis 或公共账号宿主合同 |
| §4 rule 9 | 配置和账号目录不写 Timeline；没有新增进程通知或历史事实 |
| §1 | 扩展现有进程 Provider，不新增 Capability/接线插件；业务留在 owner，App Server 通用入口仅装配与路由 |
| ADR 002 D1 | 跨 Session 配置和目录归进程；当前分支已采用配置归原 owner；临时草稿归客户端 |
| ADR 002 D2 | 复用管理 namespace、现有 resolver/host；不加注册表、Management、订阅或轮询协调层 |
| ADR 002 D3 | Rpc 不硬依赖 Provider；可选账号插件或查询不可用只影响所属能力，不加核心必需依赖 |
| ADR 002 D4 | 配置落 owner 的业务存储，更新不 reload；用途策略归消费 owner；loader 仅保留结构配置 |
| ADR 002 D5 | §2–§4 用具名服务、DTO、错误和取消定义合同；Rust 现有类型是绑定，不作为唯一规范 |

## 9. Ablation review

不增加目录副本、临时 Provider、模板实体、用途路由表、共享 credential 服务、全局变更日志、
新的事件总线或客户端配置数据库。复用 ModelEntry/ModelEdit、用户覆盖集合、CAS、有效模型投影、
已有账号入口、S4 和 F3，避免为功能对齐复制原 Desktop 的内部层次。

只增加一种管理读取，合并已有列表与账号公开条目并提供安全编辑字段；模型目录与选择复用
已有读取。create/update/probe 分别承担 identity 分配、局部写入和无持久化网络探测。
Disconnect 复用 update，重读与现有 CAS 满足本次管理流程。
以上结构各自对应用户流程或安全/一致性边界；其余可复用部分不新增抽象。
