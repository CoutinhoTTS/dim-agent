# Provider management

Status: **Draft proposal; not Ready and not implemented**. Extends the existing process
`ProviderPlugin`. The implemented S4 [`auxiliaryModel`](../appserver/auxiliary-model.md)
remains complete and unchanged. Tracking: [#329](https://github.com/arcships/dim/issues/329)
and the [roadmap](../roadmap.md).

本方案的交付标准是原 Dim Desktop 的完整 Provider 用户流程能经同一 App Server 被
Desktop 与 CLI 使用。新增方法、字段、通知和存储变更均为提案；客户端不能据此调用尚未发布的接口。
当前行为以源码和既有合同为准。设计确认、机制交付、Provider 实现与客户端验收分别记录。

## 1. Scope and current implementation

功能基线见 [Providers and models](../../feature-list.md#providers-and-models)。原 Desktop
参考实现固定在 `8cd58655ddd9`，核对入口为 `ConfigurationSettings.vue`、`model-providers.ts`、
`provider-rpc-service.ts`、`provider-gateway.ts` 与 shared `protocol/rpc.ts`；这些参考文件不成为
本仓库的依赖。基线包括创建前探测、连接管理、模型编辑、账号流程、账户用量和模型选择。

下表按 `main` @ `a6c5aa2b` 的实现区分已有能力和缺口：

| User flow | Implemented | Proposed completion |
| --- | --- | --- |
| 列表与编辑详情 | `provider.list` 返回静态记录摘要；托管目录另经 `account.providers`；`provider.catalog` 返回有效模型 | 提供统一的管理列表和安全编辑详情，包含来源、账号 authority、操作能力、配置状态和逐条错误 |
| 连接配置 | `provider.save` 可保存认证、URL、headers、代理、超时和启用状态 | 保持旧方法；新增创建与局部更新入口，避免客户端重建包含不可回读秘密的整条记录 |
| 断开与删除 | 自定义 Provider 真删除；内置 Provider 的 `delete` 仅禁用 | 明确区分禁用、移除静态凭据、删除自定义记录和账号登出 |
| 创建前获取模型 | `provider.refresh` 要求已保存的 Provider | 用未保存的连接草稿探测，不创建临时 Provider，不写入凭据或目录 |
| 模型管理 | 已有刷新、启停、可见字段编辑和重置；编辑要求模型存在于默认列表 | 新增用户模型、删除仅用户添加的模型；默认模型、用户覆盖和用户新增模型保持不同语义 |
| 模型能力 | 已有 limits、tool calling、vision、reasoning 和媒体能力编辑 | 复用 `ModelEdit`；补齐管理来源/操作说明以及 New API 的显式路由编辑 |
| 模型偏好 | 会话创建和 `session.model.update` 接受 `ModelPreferences`；F3 已交付 | 保存、回读与清除按模型的用户偏好，并接通客户端显式采用流程 |
| 账号 | 原生 Codex/Grok 与可选托管账号已有登录、恢复和登出合同 | 统一呈现状态和可执行操作；登录仍由原账号 owner 执行，状态变化可使管理视图失效 |
| 账户用量 | 无原 Desktop 账户用量管理接口 | 查询、刷新、未保存设置探测和设置 CAS；余额、额度与模型 token usage 分开 |
| 同步 | 管理调用有回复；`management.progress` 只服务单次调用 | 增加连接订阅与提交后的失效通知；断线和 owner 重载后重新订阅、读取 |
| 旧配置导入 | 已有显式只读导入和逐条结果；部分非空字段拒绝 | 覆盖持久模型偏好、用量设置、活动模型等基线字段；保持源字节、冲突诊断和重复导入保护 |

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
只负责通用入口及通知传递，不能加入 Provider 分支。已有实现位于 App Server 的 Provider
模块，不将新业务逻辑放进 `connection.rs`、`protocol.rs` 或客户端网关。

| Owner | State and responsibility |
| --- | --- |
| Provider | Provider 记录、连接配置、用户模型/覆盖、显式禁用集合、默认选择、按模型偏好、账户用量设置；有效模型与管理投影；本代任务和订阅 |
| Account owners | 登录尝试、账号状态、token、刷新策略和凭据租约；Provider 只能通过公开服务调用 |
| SqliteStorage | 原子事务、格式门禁及受支持格式迁移；不解释 Provider 的业务字段 |
| Rpc/Connection | 本地准入、namespace 路由、连接身份、通用通知声明与投递、连接关闭 |
| Session/branch configuration owners | 通过已有配置命令采用模型与参数，保存其必需 Timeline 事实 |
| Clients | 草稿、搜索、模板选择与展示；提交用户意图，消费最终投影；不保存第二份 Provider 配置 |

最终对外面如下；`ProviderService` 的 Rust 方法不是额外的跨语言公共 API：

| Surface | Consumers | Contract |
| --- | --- | --- |
| `managementMethods`, process, key `provider` | 本地 v3 Desktop/CLI；受信任进程调用者 | §3 的可序列化方法、错误与声明的通知；沿用现有 `describe/call` 绑定 |
| `account`、`account.codex`、`account.grok` 管理 namespace | 客户端账号交互 | 保持各自 owner；Provider 管理视图只返回所属 namespace、支持的登录方式与状态 |
| `ProviderConnectionResolver`、`ModelSelectionHost` | 现有原生模型调用与选择入口 | 保持现有类型合同；解析当前连接，不向终端 RPC 返回凭据 |
| `auxiliaryModel.complete`，process，无实例键 | 无法持有凭据的消费插件 | 保持 S4 的具名数据合同；调用者显式提供模型，不增加用途策略 |
| `provider.changed`，连接通知 | 本地管理客户端 | §4 的提交后失效或订阅结束提示；不含完整配置，不进入 Timeline |

内部服务只在真实跨语言消费边界提供具名绑定；不把所有内部函数逐一导出。
可选账号插件保持[公开宿主边界](../../architecture/development.md#managed-provider-boundary)，
具体账号体系、realm 和 token 流程不进入公开仓库。
发行版账号限制由原策略 owner 提供；Provider 不硬编码私有发行版标识，客户端不自行推导该策略。

## 3. Proposed management contract

### 3.1 Read models

沿用现有 `ProviderModel`、`ModelEntry`、`ModelEdit`、`ModelPreferences`、`StaticAuth` 和
`ModelNetwork` 的单一定义。管理视图只增加必要的业务说明，不从可见字段重建另一种模型配置。

| Method | Request | Reply and meaning |
| --- | --- | --- |
| `provider.management` (new) | `{}` | `{drivers, providers: ManagementProvider[], errors: ProviderRecordError[]}`；包括静态与已装载账号 owner 公布的 Provider，不按“已登录”过滤 |
| `provider.get` (new) | `{providerId}` | `{provider, connection, models, preferences}`；一次读取该 Provider 的同版本配置快照；`models` 使用有效 `ProviderModel`，附管理来源和操作说明；`preferences` 按 modelId 保存 |
| `provider.selection` (existing) | `{cwd?}` | 保持现有 `{selection}`，全局与工作区选择分开；其 version 不能用作 Provider 记录的 version |
| `provider.catalog` (existing) | `{providerId}` | 保持现有 `{catalog}`，供选择和执行消费者使用 |

`drivers` 是当前宿主允许自定义创建的 `{driverKind, authTypes, remoteModelList}` 列表，来源为
同一 driver 实现；不提供当前无法执行的选项，不增加动态表单 schema 或第二份 driver 注册表。

`ManagementProvider` 包含现有摘要、`version`、来源和 credential authority，增加
`status`（`connected` / `disconnected` / `disabled` / `error`）、公开诊断和 `actions`。
其中 `version:null` 仅表示账号 owner 已公布该 Provider、但本地目录记录尚未建立，不能用 0
冒充记录版本；此时只开放相应登录/刷新动作。详情以 `catalogState` 区分 not_loaded / ready /
error，只有 ready 才提供 models；真实空目录是 ready 加空数组，读取错误不是空目录。
每个 action 是 `{available, reason?}`，覆盖 editConnection、toggle、disconnect、delete、
probe、refreshModels、createModel、usage 和支持的账号动作；模型操作区分 toggle、edit、reset、delete。
`reason` 使用稳定代码，客户端展示信息，不从 providerId、名字或 HTTP 状态推测权限和能力。
服务端在调用时仍检查当前业务状态；action 不是授权凭证或写入承诺。

模型来源区分默认目录项、默认项上的用户覆盖和仅用户新增项。模板从已读取的有效目录派生，
以 `{providerId, modelId}` 标识来源；复制的是完整公开 descriptor，不保存模板引用，不另打包目录。
用户新增/覆盖的来源需要持久保留；刷新移除默认项不能把原覆盖误判成用户新增项。来源只说明
同一 ModelEntry 的管理语义，不复制一套模型字段；具体记录形状属于 §6 的格式决定。

`connection` 返回非秘密配置与秘密配置状态。凭据、secretRef、header 值和代理认证内容不回传；
headers 只公开名称，代理只公开不含认证的地址及认证已配置标记。该地址只用于显示，不能自动写回。
“已保存”是独立状态，不能用假 token 或占位字符串填入待保存字段。

列表允许局部失败：一个坏记录或账号错误进入 `errors`，健康 Provider 仍可读、可用。
详情返回该条明确错误，不能用空目录或未配置状态掩盖错误。聚合视图不承诺所有账号的跨 owner
原子快照；`version` 只保护所属 Provider 记录，账号状态还受其 owner 的 account/epoch 合同保护。

### 3.2 Connection and model writes

| Method | Request | Result / semantics |
| --- | --- | --- |
| `provider.create` (new) | `{displayName, connection, models: ModelEntry[], enabled}` | 管理详情；服务端分配不可复用的自定义 providerId，不从显示名称生成 identity |
| `provider.update` (new) | `{providerId, expectedVersion, patch}` | 更新后的管理详情；patch 仅含用户改变的配置/启用字段；省略保留，写入前校验并在一个事务中提交 |
| `provider.disconnect` (new) | `{providerId, expectedVersion}` | 静态内置 Provider 原子清除主凭据并禁用，保留模型与用户配置；自定义项用 delete；账号项用其 owner 的 logout |
| `provider.delete` (existing) | `{providerId, expectedVersion}` | 保持旧合同；新客户端仅对允许 delete 的自定义项调用，删除及选择清理同事务；内置项的旧调用仍只禁用 |
| `provider.probe` (new) | `{providerId?, expectedVersion?, connection}` | `{models: ProviderModel[]}`；§3.3 的只读网络探测 |
| `provider.models.create` (new) | `{providerId, expectedVersion, model: ModelEntry}` | 更新后的 catalog；用户新增项写入现有覆盖集合，重复 ID 明确拒绝 |
| `provider.models.delete` (new) | `{providerId, expectedVersion, modelId}` | 更新后的 catalog；只删除用户新增项，默认项使用禁用或 reset；相关偏好、路由和失效选择在同事务处理 |
| `provider.models.edit` (existing, extended) | 现有请求；edit 增加 `endpoint` 操作 | 保留现有字段 patch 与隐藏元数据；支持 New API 显式路由 override 的 set/clear |
| `provider.models.reset` / `set_enabled` (existing) | 保持现有请求 | reset 仅针对存在默认配置的项；启停覆盖完整有效目录，包含用户新增模型 |
| `provider.refresh` (existing) | 保持现有请求 | 更新远端默认目录，不覆盖用户模型、用户配置、偏好或显式禁用；保留当前 admitted-snapshot 冲突检查 |

`provider.save` 为现有客户端保留，不要求新客户端拼接其整条 `ProviderInput`。
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
| `provider.usage.get` (new) | `{providerId}` | `{supported, status, snapshot?, error?}`；读取当前公开用量状态，不发起隐式网络刷新 |
| `provider.usage.refresh` (new) | `{providerId}` | 同上；Provider 自持查询任务，结果不能越过账号/连接代次发布 |
| `provider.usage.probe` (new) | `{providerId, expectedVersion, settings}` | 用量快照；验证未保存设置，不写设置或查询缓存 |
| `provider.usage.configure` (new) | `{providerId, expectedVersion, settings}` | 管理详情；显式 null 清除设置，秘密字段仍使用保留/替换/清除操作 |

保存模型偏好不改变已经采用的 Session 配置。客户端读取 Provider 保存的标准
`ModelPreferences` 后，按用户操作显式传给现有 `session.create` 或 `session.model.update`；
不解析 provider-specific request options。保持 [F3](../model-configuration.md) 的原合同：
显式目标省略偏好仍从目录默认开始，无目标更新只改变当前分支提供的参数，不偷偷读取持久偏好。
自动标题、子代理等消费方仍自行决定其模型及参数，不自动继承聊天偏好。

用量快照包含实际供应方支持的余额、额度与使用量，字段带单位、币种、统计区间和采集时间；
不存在的指标省略，不用零补齐。查询状态为 not_loaded / ready / stale / error；刷新失败可保留
同账号同连接的旧快照并标 stale，不能把另一账号的快照当作当前值。模型调用 token usage
继续走原有事件，不混入账户余额。此快照缓存可丢弃；配置才是持久业务状态。

普通 Provider 的查询实现在其现有 driver/业务模块；托管账号的供应方查询仍归账号插件。
托管服务需显式协商可选的 usage capability、查询与设置 DTO；不假定当前 host API 2 已提供它，
也不向旧插件盲发新调用。具体版本协商在 Ready 前与账号 owner 确认。缺少适配支持只使 usage
不可用，不阻断登录、模型目录和请求。查询只在受信任宿主内使用现有请求内凭据，不新增公共
credentials 服务或把 bearer 交给客户端。

### 3.4 Errors, cancellation and compatibility

新管理方法的业务错误使用现有 JSON-RPC `{code, message, data:{appError}}`，增加稳定的
`provider_not_found`、`provider_conflict`、`provider_invalid_config`、`provider_unsupported`、
`provider_auth_required`、`provider_auth_failed`、`provider_transport_failed`、
`provider_cancelled` 与 `provider_failed` 分类。Provider 在边界映射类型错误，不解析内部字符串
决定分类；公开 message 为安全诊断，不拼接 token、代理认证、header 值或原始响应正文。
沿用 Rpc 的 `service_reloading` / `service_unavailable`，不在 Provider 内模拟 owner 就绪。

旧方法保留当前请求、回复及错误兼容；新方法通过 `hello.features` 的实际方法集合发现。
修改旧 DTO 的可选字段必须写清能力协商，不能仅因为连接是 v3 就发送扩展字段：
`provider.get` 与本方案的 endpoint 编辑、详情合同作为同一可用接口单元发布；watch 仍独立
按其方法集合发现，不以 get 存在推断通知已实现。
旧客户端不被要求理解新管理回复；新客户端缺少方法时显示版本不满足，不能回退到直接改库。

调用沿用 owner 的 admission、任务追踪和 drain。客户端断开只丢失回复，不撤销已提交写入；
不确定结果先重读，不盲重放。网络任务使用 Provider 的取消 token；释放网络/凭据资源后结算。
业务配置改变不 reload loader 行。并发保存、刷新、账号切换和探测发布都检查其所属配置版本或
账号 epoch；版本不匹配拒绝，不覆盖用户较新的修改。

## 4. App Server changes and notifications

| Area | Required change | Boundary |
| --- | --- | --- |
| Provider owner | §3 的类型化业务方法、统一投影、偏好/用量与模型生命周期 | 复用现有存储、driver 和有效模型投影，不另造注册表 |
| Rpc/Connection | 完成既有 callChannel.notify / closed 及 notification 声明的通用子集 | 机制所有者单独实现与验收；不识别 Provider DTO，不新增每方法权限等级 |
| Account owners | 公共账号状态失效通知、可选 usage 协商 | 凭据租约与账号 generation 仍由原 owner 决定 |
| Storage | 新业务字段与迁移的显式格式决定 | 保留精确 schema 门禁和源字节保护，不静默复用 v18 |
| Session entry | 复用现有 create/model.update | 不增加 Provider Timeline 事件，不改 Prompt 配置采用规则 |

通知是管理视图失效提示，不是持久事件、变更日志或新事实来源。Provider 的写操作通知自己
拥有的 watch；不为尚无消费者的用途增加公共 Provider 事件。本地客户端通过下列提案订阅，
不能订阅任意 rutis 内部事件：

| Method / notice | Data | Lifetime |
| --- | --- | --- |
| `provider.watch` (new) | `{}` → `{watchId}` | 绑定发起请求的本地连接和当前 Provider 代次；调用方不能传 connectionId |
| `provider.unwatch` (new) | `{watchId}` → `{}` | 只能释放本连接的订阅；未知或已释放 handle 幂等结束 |
| `provider.changed` (new notice) | `{watchId, kind:"invalidate", providerIds:[], selectionScopes:[]}` | 提交后发送；标识需重读的条目/选择，不含配置、账号身份或秘密 |
| `provider.changed` (watch ending) | `{watchId, kind:"closed"}` | 正常 owner 卸载前结束订阅；连接断开则由通用关闭机制释放 |

同一连接同一代只保留一个 watch；重复 watch 返回当前 handle。`watchId` 是运行时句柄，不持久化，
不复用为 Provider identity。注册先于 watch 回复；通知可能先于回复到达，因此客户端先安装接收器。
Provider 写事务和选择事务各自在提交后发布其失效，两个 owner 的操作不伪装成同一事务。
账号 owner 的状态变化由其公开事件触发目录失效；Provider 不读取其 token 存储。

本方案要求各账号 owner 增加一个公共进程结果通知 `account.state_changed`，channel 为
`account/<providerId>`，数据仅 `{providerId}`。原生账号 owner 和托管账号绑定在其状态变化、
绑定/解绑或凭据 epoch 更新完成后发出；Provider 订阅后重读公开状态并使相关管理视图失效。
它不携带 token/账号身份，不重新实现登录，也不为无变化的 token 检查发事件。此事件及其
托管绑定属于账号 owner 的待实现合同，不假定当前接口已具备。

接收队列不能静默丢弃 invalidate/closed 后继续维持“已同步”：队列满时通用通知机制关闭
受影响连接，调用方得到 gone，客户端重连同步。不另增重同步消息协议。
`management.progress` 的可丢弃语义不能复用为通知保证。
正常 Provider 重载先结算已接受工作，再结束 watch；新代必须重新 watch。服务端重启、断线或
无法确认订阅连续性时，客户端丢弃订阅句柄并重读；不增加历史 replay 或全局变更 sequence。

### Requirements raised to mechanism owners

| Item | Connection-bound notifications |
| --- | --- |
| Blocked by | [session-boundary A.2](../contracts/session-boundary.md#a2-call-channel) 的 notify、closed 和通知声明尚未实现；现有管理方法不能在回复后向所属连接发送已声明通知 |
| Existing mechanisms considered | progress 仅在调用期间有效且可丢弃；C6 是 Session 观察，不能承载进程状态；轮询不提供提交后的变更通知；Provider 直接持有 socket 或 ConnectionHub 会绕过通用入口 |
| Generic capability | Rpc-owned callChannel 的连接身份、namespace 声明校验、notify、closed，以及有界队列溢出时关闭受影响连接；复用现有 named method binding，支持原生与外部语言 owner |
| Ordering, batch, interrupt, reload | 不分配 Timeline 序号，不改批次；仅提交后发送；连接关闭释放订阅而非回滚写入；owner 代次结束可被调用方识别；通知与单次调用结果分别结算 |
| Owner / tracking | Rpc/Connection 负责线；[#329](https://github.com/arcships/dim/issues/329) 记录需求，进入 Ready 前确定机制子项及负责人；机制未交付前不得发布 watch 能力 |

| Item | Storage format and mutation revisions |
| --- | --- |
| Blocked by | [native format](../appserver/native-storage-format.md) 的精确 v18 门禁及旧记录语义不包含新增业务字段；旧客户端可通过 legacy save 删除/重建同 ID，单纯从 1 重置 version 无法拒绝旧 CAS |
| Existing mechanisms considered | 现有事务、记录 version 和 import marker 可复用；import marker 不覆盖所有新记录，不能冒充通用版本历史；Timeline sequence 不属于进程配置，不能借用 |
| Generic capability | 原生格式的显式升级事务与精确校验；Provider/selection 业务存储的版本在删除重建后也不复用，所有新旧写入口共用该版本分配规则，客户端把 version 视作不透明 CAS 值 |
| Ordering, batch, interrupt, reload | 版本分配、业务写入、关联选择清理和导入标记在所属事务内提交；中断不产生部分记录；重启不重置版本；不改变 Timeline/Dispatcher 或 Session 恢复 |
| Owner / tracking | 存储负责线与 Provider owner；[#329](https://github.com/arcships/dim/issues/329) 记录，Ready 前确认实际格式号、分配规则和迁移向量；不增加 core 全局时钟 |

本提案只要求完成已有 A.2 的通用通知子集，不要求 takeFinalReply、RemoteGateway、rutis 修改或
新的事件总线。此需求和存储格式决定是独立 owner 的交付，不是 Provider 可以自行绕过的前置。

## 5. Client integration flow

```text
Desktop / CLI                Local App Server               Provider / account owners
    | hello v3, inspect features     |                                |
    | provider.watch --------------->| register connection watch       |
    | provider.management ---------->|------------------------------->|
    | provider.get / selection ----->|<----- safe typed snapshots -----|
    | edit local draft               |                                |
    | create / probe / CAS update -->|------------------------------->|
    |                                |            validate + commit   |
    |<---------- result ------------|<-------------------------------|
    |<----- provider.changed --------|<----- after-commit invalidate --|
    | reread affected snapshots ---->|------------------------------->|
    | create/update Session with explicit model + preferences          |
    | disconnect / reconnect: watch again, then reread                  |
```

1. 使用同一 product home 的本地 socket / named pipe，协商 v3；WebSocket 和配对设备不因此获得管理权限。
2. 检查所需的实际方法集合；先安装通知接收器并 watch，再读管理列表、选择和所选详情。
   读取期间发生失效，置 dirty 并在当前读取结束后再读一次，避免 snapshot/watch 竞态。
3. 未配置项先编辑连接并 probe，或手动添加模型；创建后使用服务端返回 identity/version。
   已有项提交 changed fields，秘密默认 keep；取消草稿不发配置写入。
4. 发生版本冲突时读取新快照，保留未提交草稿并提示用户重新确认；不能用新 version 自动覆盖。
   单条错误不清空整个列表；过期请求不覆盖当前选择、当前草稿或较新的读结果。
5. 模型新增、编辑、启停和刷新分别提交其语义；当前搜索仅过滤展示，全部模型操作作用于完整目录。
   模板选择复制公开完整 descriptor；媒体声明不能被客户端等同于执行支持。
6. 登录按详情提供的账号 namespace/方式调用既有账号方法；浏览器打开和回调交互属于客户端，
   凭据与恢复归账号 owner。断开、禁用、删除和 logout 不互相冒充；失败不能显示为成功。
7. 用户选模型时通过 provider.select 保存默认选择；要改变当前 Session，再显式提交已有
   session.model.update。两次操作分别报告结果，保存默认不代表已配置当前 Session。
8. 读取保存的 ModelPreferences 并显式采用；用途配置从对应消费插件读取。用量面板读取状态，
   用户刷新才发 refresh，设置探测不保存；过期数据明确标注。
9. 自己提交成功也以返回值/重读为准。其他客户端失效通知触发重读，不覆盖正在编辑的草稿；
   watch closed、断线和服务端重启后重新订阅并完整同步，不依赖通知 replay。

界面保持原有组件与布局，下面仅规定必须区分的交互状态，不是替换产品视觉设计：

```text
Providers                         Provider detail
[ Search... ] [+ Add] [Refresh]    Name / source / connection status
  Builtin A         Connected     [Edit connection] [Disable] [Disconnect]
  Custom B          Disabled      Credential: Stored  [Keep | Replace | Clear]
  Account C         Signed out    Models [Search...] [Add] [Refresh]
  Broken D          Error         [ ] All models
                                  Model      Enabled   [Edit] [Reset/Delete]
                                  Usage: status + collected time [Refresh]
                                  Unsaved draft: [Cancel] [Save]
                                  Conflict: reload current / retain draft
```

CLI 提供等价功能，不必复制 Desktop 布局。两端均不能根据硬编码 Provider 名称推测操作能力，
不能通过“暂存真实 Provider 再删除”模拟 probe，也不能把未保存值写入客户端持久配置以绕过协议。

## 6. Persistence, migration and settlement

默认目录、用户覆盖/新增、禁用集合与连接保持原有唯一记录；增加按模型偏好、显式路由和用量
设置所需字段。默认选择继续有独立 scope/version；用途配置归其消费 owner。用量缓存、watch、
网络 client 与已解析凭据不是新的持久事实。模板、页面搜索、临时探测与编辑草稿不存到服务端。

**格式提案：需要新原生格式，不把不兼容记录继续写为 v18。** 旧二进制可能忽略新增字段后重写，
丢失偏好、用量设置或路由语义。存储负责线须在 Ready 前确定实际格式号、精确 schema、记录
校验与 v18 升级事务，并更新 [native format](../appserver/native-storage-format.md)；本 PR
不占用未来版本号，也不把格式迁移标为已交付。未知版本保留原字节，不回退为空状态。

旧 TypeScript 配置通过现有显式只读导入边界迁移，逐条报告 imported/already_imported/rejected：

- 迁移连接、用户模型与覆盖、显式禁用、模型偏好、用量设置，以及 global/workspace 选择；
  activeModelId 仅作 Provider 的最后选择提示，不能覆盖已有显式 scoped selection。
- 不支持的 retryConfig/capabilityDefaults 等字段必须决定等价映射或明确拒绝，不能丢字段后报成功；
  Ready 前逐字段冻结导入清单与冲突优先级。
- 已有目标记录不被自动覆盖；失败条目的来源、字节与 import marker 保留。重试不复活用户已删除的项。
- OAuth grants 不由通用导入复制；由账号 owner 显式恢复/重新登录。账户用量旧快照不跨账号导入。

自定义新 identity 不复用，已删除旧 identity 不通过 import 自动复活。现存 legacy save 的
expectedVersion 请求合同保留，但所有写入口必须采用删除重建后不复用的业务版本；不新增
仅对新客户端有效的并发保护。远端刷新/用量查询继续比对其 admitted 配置及账号代次，
不能仅靠记录 version 判断请求期间账号是否相同并接受迟到结果。

Provider 在 apply 中立即登记清理，发布后再启动业务观察。关闭准入、取消网络 I/O、释放
凭据租约、结算已接受调用和 watch，再移除服务。客户端 detach 不关闭 Session；Provider
设置变化不重放历史请求，也不修改已采用的分支配置。

## 7. Acceptance and delivery

| Gate | Required evidence |
| --- | --- |
| Design Ready | §4 通知子项及 owner、存储格式/迁移、账号 usage 协商与逐字段导入映射确认；对应共享合同同步，仍不宣称实现 |
| Safe configuration | 改超时保留代理认证，改一个 header 保留其他 header；keep/set/clear 与 secretRef 流程；日志、错误、RPC、通知中无秘密 |
| Model lifecycle | 内置 Provider 新增用户模型并实际调用；默认项 edit/reset、用户项 delete；刷新/重启保留覆盖与禁用；New API 路由在真实请求生效 |
| Probe | 未保存配置可获取模型；取消/失败无 Provider、凭据、选择或目录写入；过期草稿回复不被采用 |
| Preferences | 保存、清除、重启回读；显式传参的新 Session/切换采用正确参数；F3 无目标更新保持原行为；用途模型不被串改 |
| Accounts and usage | 原支持的登录方式、取消、恢复、刷新和登出完整；查询设置 CAS、probe 不保存、stale 展示；账号切换/迟到结果不泄漏旧快照 |
| Multiple clients | 先 watch 后 read 的竞态、两个客户端 CAS 冲突、队列满、断线、owner 正常重载与服务端重启；无法继续同步时显式恢复 |
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
| §4 rule 4 | Rpc 只识别 namespace、声明和连接，不解释 Provider；Timeline/Dispatcher 不增加业务字段或分支 |
| §4 rule 5 | 关联的模型/偏好/路由/选择清理同事务；过程级变更不进入 Timeline；现有 Session 原子采用不变 |
| §4 rule 6 | 管理读取、配置和通知不进入模型上下文；实际 Session 变化只经既有采用路径 |
| §4 rule 7 | 账号经公开服务和事件；不读 token 文件、不跨写内部状态；有效目录由 Producer 保证 |
| §4 rule 8 | 通用连接通知和存储格式需求见 §4 的五项表；账号服务由其 owner 决定；不改 rutis，不绕过未交付机制 |
| §4 rule 9 | 配置、账号目录和 watch 不写 Timeline；进程失效通知不能冒充 Session 广播或历史事实 |
| §1 | 扩展现有进程 Provider，不新增 Capability/接线插件；业务留在 owner，App Server 通用入口仅装配与路由 |
| ADR 002 D1 | 跨 Session 配置和目录归进程；当前分支已采用配置归原 owner；临时草稿归客户端 |
| ADR 002 D2 | 复用管理 namespace、现有 resolver/host 与 callChannel；不加 ProviderRegistry、Management 或轮询协调层 |
| ADR 002 D3 | Rpc 不硬依赖 Provider；可选账号/usage 不可用只影响所属能力，不加核心必需依赖 |
| ADR 002 D4 | 配置落 owner 的业务存储，更新不 reload；用途策略归消费 owner；loader 仅保留结构配置 |
| ADR 002 D5 | §2–§4 用具名服务、DTO、错误、取消和声明通知定义合同；Rust 现有类型是绑定，不作为唯一规范 |

## 9. Ablation review

不增加目录副本、临时 Provider、模板实体、用途路由表、共享 credential 服务、全局变更日志、
新的事件总线或客户端配置数据库。复用 ModelEntry/ModelEdit、用户覆盖集合、CAS、有效模型投影、
已有账号入口、S4 和 F3，避免为功能对齐复制原 Desktop 的内部层次。

保留两种新读取，因为列表需要统一状态与动作，编辑详情需要同版本的完整有效模型和安全字段
状态；保留 create/update/probe，因为 identity 分配、局部写入和无持久化网络探测是不同业务操作。
保留通用连接通知，因为单次 progress 与 Session 广播无法覆盖本地管理连接的存活期。
以上结构各自对应用户流程或安全/一致性边界；其余可复用部分不新增抽象。
