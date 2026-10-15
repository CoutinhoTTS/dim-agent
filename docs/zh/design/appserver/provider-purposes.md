# Provider 用途策略与 AuxiliaryModel

状态：草案（待评审）。路线图项 S4（[#53](https://github.com/arcships/dim/issues/53)）。
总体设计：[附录 B.2 Provider 行](../overview.md#b2-app-server-进程插件)、
[§6.2 约束 14](../overview.md#62-约束)。作用域：
[ADR 002](../../adr/002-plugin-scope.md)（进程级，放在现有的 Provider 插件内）。

## 1. 摘要

Provider 只新增两样东西：

1. **用途策略。** 用户可以给某个用途 key（如 `session-title`、`compaction`）固定一个模型。
   策略存放在 Provider 自己的存储里，只有一条写入路径：Provider 的类型化写操作，对客户端
   暴露为 `provider.purposes` / `provider.purpose.set`。
2. **`auxiliaryModel` 服务。** 一个异步方法 `complete(purpose, inherit, request)`，在任何
   Session 之外发起一次非流式模型调用：选模型（策略条目，否则调用方给的 `inherit` 模型），
   在 Provider 内部解析连接，调用现有的 `ModelHost`，返回文本。它不写任何 Timeline。

S4 "完成标准"里的使用者：SessionTitle（P4，用途 `session-title`）和生产压缩宿主
（F1，用途 `compaction`）。Collaboration（`collaboration.member`）和 Modality
（`modality.<category>`）以后通过同一套读写使用策略，不属于 S4 的验收。不新增插件、
不新增 Timeline 事件、不改基础设施。

## 2. 结构

```
 进程作用域
 +-------------------------------------------------------------------+
 |  Provider（现有插件）                                             |
 |   ProviderService ----- provider_selections（SQLite，v13）        |
 |     selection/select     scope = global | workspace:<cwd>         |
 |     purpose/set_purpose  scope = purpose:<key>        （新行）    |
 |   ProductResolver（连接与凭据，不变）                             |
 |   AuxiliaryModel  --使用--> ProductResolver、ModelHost            |
 |   managementMethods "provider"：+ provider.purposes、             |
 |                                   provider.purpose.set            |
 +--------------------------^------------------^---------------------+
        Aimux：ModelHost ---'                  |
                                               | complete(purpose, inherit, request)
 Session 作用域                                |
   SessionTitle（Deps：auxiliaryModel）--------+
   F1 压缩宿主（每次调用时查找）---------------'
   Collaboration（以后）：ProviderService.purpose("collaboration.member")
```

依赖指向核心：Provider 新增对 Aimux `ModelHost` 的硬依赖（两者都是核心，在产品配置中
总是同时存在）。Provider 不点名任何使用者。

## 3. 数据与状态

**用途 key。** 由使用它的使用者拥有并写进自己的文档（约束 14）；Provider 把它当作不透明
字符串，不维护已知 key 的登记表。语法在 Provider 的入口（管理方法、类型化写操作、
`complete`）检查：一到四段，以点分隔，每段 `[a-z0-9-]+`，总长不超过 64 字节。第一段是
使用者自己的前缀。

| Key | 所有者 | 调用类型 |
| --- | --- | --- |
| `session-title` | SessionTitle | `auxiliaryModel.complete` |
| `compaction` | F1 压缩宿主 | `auxiliaryModel.complete` |
| `collaboration.member` | Collaboration | 对话型：成员 Session 的模型 |
| `modality.<category>[.<operation>]` | Modality | 非对话型（Modality 自己的路径） |

**策略条目。** `{purpose, providerId, modelId, version}`，只有全局一级（没有使用者需要按
工作区区分用途）。没有其他参数：功能清单里列了按用途设置推理强度，但还没有设计（开放问题 1）。

**存储。** 放在现有 `provider_selections` 表中，`scope = "purpose:<key>"`。行的形状正好
是一个带 CAS 版本的模型选择，因此不需要新类型、新表或新格式版本；现有 scope 是 `global` 和
`workspace:<绝对路径>`，不会冲突。现有行为正好适用：删除 Provider 会删除指向它的选择行
（条目消失，用途按下文回退）；模型暂时不可用时保留该行。实现 PR 在
[native-storage-format](../../../design/appserver/native-storage-format.md) 中记录新的
scope 取值。

**为什么用 Provider 的存储而不是 Settings 源。** ADR 002 D4 两者都允许。Settings 源在 S8b
加入通用的 `settings.update` 之后会多出第二个写入者，而且 Settings 无法对照 Provider 的目录
检查目标模型。总体设计和 Modality 都要求"一条写入路径"；默认选择是同一类数据，已经存在这里，
用的是同样的 CAS 写入。不需要变更通知：每个使用者都在使用时读取策略。

**运行时状态。** 无。`complete` 不缓存，除 Provider 现有的任务跟踪外不记录进行中的调用。

## 4. 接口

### 4.1 语言中立（ADR 002 D5）

`auxiliaryModel`，进程作用域，无实例 key，由 Provider 提供。

| 方法 | 模式 | 请求 | 回复 |
| --- | --- | --- | --- |
| `complete` | async | `{purpose, inherit: {providerId, modelId}, request: {system?: string, messages: Message[], maxOutputTokens?: u32}}` | `{text, stopReason: "end_turn" \| "max_tokens" \| "stop_sequence", usage?: {promptTokens, completionTokens, totalTokens}}` |

- `messages` 使用现有可序列化的 `Message` 类型（dim-agent-types）。SessionTitle 发送一条用户
  文本消息；压缩宿主发送要总结的历史加上它的指令。不提供工具；丢弃 thinking 输出；只有文本块
  组成 `text`。
- `inherit` 必填：用途没有条目时使用的模型。SessionTitle 传 Session 的模型（即它的 A1 提示，
  由可选改为必填）；压缩宿主传被压缩请求的模型。
- `maxOutputTokens` 限制回复长度（让标题生成保持便宜）；缺省为模型上限，并截到模型的
  `max_output_tokens`。
- 非流式：两个使用者都需要完整文本。
- 取消：使用者丢弃调用（或发送桥接协议的取消请求）。服务本身没有超时，预算归使用者
  （SessionTitle：30 秒）。
- 错误是数据 `{code, message}`：`invalid_request`、`model_unavailable`、`failed`、
  `cancelled`（§6）。

客户端管理方法（命名空间 `provider`，与 `provider.select` 一样只服务本地传输）：

| 方法 | 参数 | 结果 |
| --- | --- | --- |
| `provider.purposes` | `{}` | `{purposes: [{purpose, providerId, modelId, version}]}` |
| `provider.purpose.set` | `{purpose, model: {providerId, modelId} \| null, expectedVersion?}` | 条目；清除后为 `null` |

`set` 先检查 key 语法，再（模型非空时）检查模型在该 Provider 目录中可选，与
`provider.select` 的检查相同；`expectedVersion` 是 CAS（缺省表示"尚无条目"）。使用者：CLI
`/settings` 的"辅助模型"页面（功能清单）和 Modality 的 `modality.configure`。

### 4.2 Rust 绑定

放在 `dim-agent-capabilities::host`，与 `ModelSelectionHost` 相邻，因为压缩端口
（`ResolveContextModel`）在那里，而且没有 Provider 的配置也可以实现它：

```rust
pub struct AuxiliaryRequest { pub system: Option<String>, pub messages: Vec<Message>,
                              pub max_output_tokens: Option<u32> }
pub struct AuxiliaryReply { pub text: String, pub stop_reason: ModelStopReason,
                            pub usage: Option<Usage> }
pub enum AuxiliaryErrorCode { InvalidRequest, ModelUnavailable, Failed, Cancelled }
pub struct AuxiliaryError { pub code: AuxiliaryErrorCode, pub message: String }
pub trait CompleteAuxiliary: Send + Sync + 'static {
    fn complete(&self, purpose: &str, inherit: ModelId, request: AuxiliaryRequest,
                cancel: CancellationToken)
        -> Pin<Box<dyn Future<Output = Result<AuxiliaryReply, AuxiliaryError>> + Send + '_>>;
}
pub struct AuxiliaryModelHost(pub Arc<dyn CompleteAuxiliary>);
```

Provider 的 `apply` 提供 `AuxiliaryModelHost`；其 `Deps` 增加 `Arc<ModelHost>`。
`ProviderService` 增加 `purposes()`、`purpose(key) -> Option<PurposeEntry>` 和
`set_purpose(key, Option<ModelId>, expected_version)`；管理方法调用它们。

## 5. 流程

### 5.1 `complete`

```
 使用者            AuxiliaryModel（Provider）  ProviderService      ModelHost（Aimux）
    | complete(p, inherit, req) |                    |                     |
    |-------------------------->| 检查 key、请求     |                     |
    |                           | purpose(p) ------->| 行 purpose:<p>？    |
    |                           |<-- 条目 | 无 ------|                     |
    |                           | 目标 = 条目或 inherit                    |
    |                           | configuration(Some(目标)) ->| 可选？配置 |
    |                           |<-- ModelSelection | model_unavailable    |
    |                           | 构造 ModelRequest（system、messages、max_tokens、无工具）
    |                           | ProductResolver.resolve -> 连接（凭据留在这里）
    |                           | invoke(request, connection, cancel) ---->|
    |                           |<------------ 块：Text、Usage、MessageStop
    |<-- {text, stopReason, usage}                                         |
```

如果流因凭据被拒而失败，且连接的 `RecoverModelConnection` 判定可恢复（托管 OAuth），
Provider 恢复一次并重试一次。没有瞬时错误重试或退避：重试归使用者（SessionTitle 在之后的
输入时重试；压缩宿主按自己的失败规则处理）。任何地方都不写事件。

### 5.2 设置策略

```
 CLI /settings -> Rpc -> provider.purpose.set {purpose, model, expectedVersion}
   Provider：语法 -> catalog_for(provider).selectable(model) -> CAS 写入或删除行
   回复：条目 | null        （无通知；下一次 complete() 读到新行）
```

### 5.3 模型选择顺序

| 步骤 | 条件 | 结果 |
| --- | --- | --- |
| 1 | 用途有条目 | 使用它。不可选（模型被禁用、不可用、目录缺失）：`model_unavailable`，**不回退** |
| 2 | 无条目（从未设置、已清除或其 Provider 已删除） | 使用 `inherit`。不可选：`model_unavailable` |

已配置的条目从不静默回退：用户选定了这些内容发往哪里，改发给别的 Provider 会让人意外。
与默认选择（会继续尝试其他候选）不同，`complete` 在步骤 1 或 2 之后只有一个候选。

## 6. 错误

| 代码 | 何时 | 重试 |
| --- | --- | --- |
| `invalid_request` | 用途 key 非法、`messages` 为空、`maxOutputTokens = 0` | 否（调用方缺陷） |
| `model_unavailable` | 选中的模型不可选，或其 Provider、目录缺失 | 用户修正策略或目录之后 |
| `failed` | 连接解析、传输、HTTP 或流错误（在一次凭据恢复之后） | 由使用者决定 |
| `cancelled` | 调用被丢弃、取消令牌触发，或 Provider 卸载（其停止令牌） | 由使用者决定 |

管理方法的错误映射与现在的 `provider.select` 相同（参数无效、模型不可用、版本冲突）。

## 7. 安全

- 凭据不离开 Provider：`ResolvedModelConnection` 在 `complete` 内部构造和使用，从不返回或
  序列化。使用者只看到文本、停止原因和用量。
- 错误消息不含秘密：`failed` 使用现有 `HostError` 的消息，它不包含凭据值；Provider 从不把
  连接格式化进消息。
- 诊断：`complete` 发出进程级 `tracing` span `auxiliary.complete`，含用途、provider id、
  model id、结果、耗时和 token 用量，从不包含请求或回复内容。使用者在自己的 Session 诊断中
  记录结果（SessionTitle：只记结果和耗时）。辅助调用不是 Session 事实，不进入任何 Timeline。
- 策略是用户配置，只由本地客户端或受信任的进程内所有者写入；配对的远程设备永远不会被提供
  `provider.*` 方法。

## 8. 与现有部件的关系

| 部件 | 关系 |
| --- | --- |
| 默认选择（`provider.select`） | 不变。`complete` 不用它，由调用方的 `inherit` 代替 |
| SessionTitle（P4） | 调用 `complete("session-title", Session 模型, …)`；可在 `Deps` 中声明 `auxiliaryModel`（扩展指向核心）。其 A.2 形状把 `inherit?` 改为必填 |
| 压缩宿主（F1） | 实现 `ResolveContextModel`；`compact` 调用 `complete("compaction", 请求模型, …)`，每次调用时查找，因为 `ContextModelHost` 是 Session 值，而没有 Provider 的配置没有 `auxiliaryModel` |
| Collaboration | 以后由它自己改：成员模型 = profile 模型，否则把 `purpose("collaboration.member")` 作为显式模型，否则沿用现在的默认选择；条目不可用时成员创建失败。"未设置"是否应表示沿用 leader 的模型，由 Collaboration 决定 |
| Modality（P3） | 通过 `set_purpose` 保存 `modality.*` 条目；非对话型连接解析和非聊天模型的目录条目是另一项 Provider 补充，不在 S4 内 |
| Workflow `workflow.stage` | 建议去掉：`stage.model` > profile 模型 > Agents 默认已经覆盖，没有功能需要单独的阶段默认值，而且 Workflow 还需要按名字读策略（#4） |
| ExternalAgent | 无特殊处理：条目可以指向 `external-agent:*` Provider 条目；ModelHost 像其他请求一样路由 |

## 9. 规则检查

| 规则 | 回答 |
| --- | --- |
| 原则 §3（依赖方向） | Provider（核心）只依赖 Aimux（核心）；使用者依赖或查找 Provider；Provider 不点名使用者。 |
| §4 规则 3（before 回调） | 不使用。 |
| §4 规则 8（基础设施不变） | 不改 Timeline、Dispatcher、事件格式、rutis 或装配；使用 `ModelHost`、`ProductResolver`、现有表和 Rpc 的 `managementMethods`。不向 rutis 提需求。 |
| §4 规则 9（只有需要的事实进 Timeline） | 策略是进程状态，经 Provider 的类型化写操作修改；辅助调用不写事件。压缩摘要由 ContextPreparation 现有的事实记录，而不是本服务。 |
| ADR 002 D2（无接线模块） | 不新增插件；服务放在模型与凭据的所有者中。 |
| ADR 002 D3（依赖） | 核心不硬依赖可选插件；F1 宿主每次调用时查找服务；SessionTitle（可选）可以依赖它。 |
| ADR 002 D4（配置） | 用途策略是业务配置，存放在 Provider 自己的存储中；变更是数据更新，使用时读取，不重载。 |
| ADR 002 D5（语言中立契约） | §4.1 是契约；§4.2 是一种绑定。 |

## 10. 开放问题

1. 按用途的推理强度（功能清单中的"辅助模型与推理强度"）：需要新列或不同的行形状，因此需要
   格式决定。推迟到设计 `/settings` 时；S4 只保存模型。
2. F1 是否需要在发送前知道所选模型的上下文窗口（以便为较小的压缩模型裁剪历史）？如果需要，
   在 F1 中增加 `resolve(purpose, inherit) -> {model, capabilities}` 读取；S4 不预先添加。
3. API key `--models` 配置（ConfiguredModels，没有 Provider 插件）是否应提供
   `auxiliaryModel`？目前不提供，因此标题会被跳过，F1 必须把服务缺失视为"压缩不可用"。

## 11. 验收（流程测试）

1. `provider.purpose.set` 后 `provider.purposes` 能往返读写并做 CAS；过期的
   `expectedVersion` 冲突；清除返回 `null`；非法 key 和不可选模型被拒绝；默认选择不受影响。
2. 对 mock HTTP provider 调用 `complete("session-title", inherit, …)`：无条目时请求发往
   `inherit`；`set` 之后发往条目；请求带 system 文本、messages、`max_tokens` 且无工具；回复是
   文本加用量。
3. 条目指向被禁用的模型：`model_unavailable`，不发请求，不回退。
4. 删除条目所属的 Provider：下一次调用使用 `inherit`。
5. 流进行中丢弃调用会取消请求；卸载 Provider 会以 `cancelled` 结束进行中的调用。
6. 回复、错误和 trace span 都不含 API key；调用后 Session 的 Timeline 没有新事件。
7. F1 压缩流程测试（在 F1 中）调用 `complete("compaction", …)` 并记录现有的压缩事实。
