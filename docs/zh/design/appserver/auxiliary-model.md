# AuxiliaryModel

状态：草案（待评审）。路线图项 S4（[#53](https://github.com/arcships/dim/issues/53)）。
总体设计：[附录 B.2 Provider 行](../overview.md#b2-app-server-进程插件)。作用域：
[ADR 002](../../adr/002-plugin-scope.md)（进程级，放在现有的 Provider 插件内）。

## 1. 摘要

Provider 只新增一个服务 `auxiliaryModel`，它只有一个异步方法 `complete(model, request)`。
它在任何 Session 之外发起一次非流式模型调用：在 Provider 内部为给定模型解析连接，调用现有的
`ModelHost`，返回文本。它不写任何 Timeline，凭据不离开 Provider。

用哪个模型由调用方决定，这个选择保存在调用方自己的业务配置里。Provider 不保存任何按调用方
区分的策略：没有用途 key，没有用途存储，也不新增管理方法。

S4 完成标准里只有一个使用者：SessionTitle（P4），它需要在自己的 Session 之外调用模型，又拿
不到凭据。不新增插件、不新增 Timeline 事件、不改基础设施。

### 1.1 为什么这么小

需求来自功能清单："Auxiliary models: configure models for auxiliary tasks"，以及设置页条目
"Auxiliary model and reasoning effort"。总体设计的早期版本把它变成了由 Provider 持有的按用途
策略（单一写入路径），还让压缩也走这个服务。对照代码，两者都超出了需求：

- 配置辅助模型是执行该任务的使用者自己的业务设置。每个使用者保存自己的选择；CLI 设置页逐个
  读取各所有者，汇总展示。没有任何需求要求由 Provider 保存它们。
- 压缩已经有自己的端口 `ResolveContextModel` → `ContextModel`
  （`dim-agent-capabilities/src/host.rs`）：上下文窗口、输出预算、token 计数和 `compact`。
  只返回文本的 `complete` 会丢掉这些信息。生产压缩宿主（F1）实现这个端口，不使用
  `auxiliaryModel`。

## 2. 结构

```
 进程作用域
 +----------------------------------------------------------------+
 |  Provider（现有插件）                                          |
 |   ProviderService：目录、默认选择（不变）                      |
 |   ProductResolver：连接与凭据（不变）                          |
 |   AuxiliaryModel --使用--> ProviderService、ProductResolver、  |
 |                            ModelHost                           |
 +-------------------------^--------------------^-----------------+
       Aimux：ModelHost ---'                    |
                                                | complete(model, request)
 Session 作用域                                 |
   SessionTitle（Deps：auxiliaryModel）---------'
     model = 其 Settings 来源 `session-title` 的 model，否则为 Session 的模型
```

依赖指向核心：Provider 新增对 Aimux `ModelHost` 的硬依赖（两者都是核心，在产品配置中总是同时
存在）。Provider 不点名任何使用者。

## 3. 数据与状态

无。Provider 不新增存储、不缓存，除现有任务跟踪外不记录进行中的调用。

## 4. 接口

### 4.1 语言中立（ADR 002 D5）

`auxiliaryModel`，进程作用域，无实例 key，由 Provider 提供。

| 方法 | 模式 | 请求 | 回复 |
| --- | --- | --- | --- |
| `complete` | async | `{model: {providerId, modelId}, request: {system?: string, messages: Message[], maxOutputTokens?: u32}}` | `{text, stopReason: "end_turn" \| "max_tokens" \| "stop_sequence", usage?: {promptTokens, completionTokens, totalTokens}}` |

- `model` 必填。调用方从自己的配置中解析出它；Provider 不回退到其他模型。
- `messages` 使用现有可序列化的 `Message` 类型（dim-agent-types），例如
  `[{role: "user", content: "…"}]`。不提供工具；丢弃 thinking 输出；只有文本块组成 `text`。
- `stopReason` 有三个取值。Rust 的 `ModelStopReason` 还有 `ToolUse`；因为不提供工具，出现工具
  停止属于提供方缺陷，调用以 `failed` 失败。
- `maxOutputTokens` 限制回复长度；缺省为模型上限，并截到模型的 `max_output_tokens`。
- 非流式：使用者需要完整文本。
- 取消：使用者丢弃调用（或发送桥接协议的取消请求）。服务本身没有超时，预算归使用者。
- 错误是数据 `{code, message}`：`invalid_request`、`model_unavailable`、`failed`、
  `cancelled`（§6）。

### 4.2 Rust 绑定

放在 `dim-agent-capabilities::host`，与 `ModelSelectionHost` 相邻：

```rust
pub struct AuxiliaryRequest { pub system: Option<String>, pub messages: Vec<Message>,
                              pub max_output_tokens: Option<u32> }
pub struct AuxiliaryReply { pub text: String, pub stop_reason: ModelStopReason,
                            pub usage: Option<Usage> }
pub enum AuxiliaryErrorCode { InvalidRequest, ModelUnavailable, Failed, Cancelled }
pub struct AuxiliaryError { pub code: AuxiliaryErrorCode, pub message: String }
pub trait CompleteAuxiliary: Send + Sync + 'static {
    fn complete(&self, model: ModelId, request: AuxiliaryRequest, cancel: CancellationToken)
        -> Pin<Box<dyn Future<Output = Result<AuxiliaryReply, AuxiliaryError>> + Send + '_>>;
}
pub struct AuxiliaryModelHost(pub Arc<dyn CompleteAuxiliary>);
```

Provider 的 `apply` 提供 `AuxiliaryModelHost`；其 `Deps` 增加 `Arc<ModelHost>`。

## 5. 流程

```
 使用者            AuxiliaryModel（Provider）  ProviderService       ModelHost（Aimux）
    | complete(model, req)  |                       |                       |
    |---------------------->| 检查请求              |                       |
    |                       | configuration(Some(model)) ->| 可选？         |
    |                       |<-- ModelSelection | model_unavailable         |
    |                       | 构造 ModelRequest（system、messages、max_tokens、无工具）
    |                       | ProductResolver.resolve -> 连接（凭据留在这里）
    |                       | invoke(request, connection, cancel) --------->|
    |                       |<------------ 块：Text、Usage、MessageStop     |
    |<-- {text, stopReason, usage}                                          |
```

失败处理与 Session 请求用同一套策略，放在一个共用函数里
（`dim-agent-capabilities::model_provider::invoke_with_recovery`，ModelProvider 也用它），
所以这套策略只有一份实现：

- **凭据：** 被拒的凭据恢复一次；恢复后再次被拒（`Terminal`）时，先由凭据所有者的
  `finish_rejected` 结算（例如托管账号被登出），再让调用失败。
- **瞬时失败：** 在共用预算内退避重试（至多三次尝试、等待合计 60 秒），提供方的
  `ModelFailurePolicy::forbids_retry` 禁止时不重试。

此外的重试归使用者（SessionTitle 在之后的输入时重试）。任何地方都不写事件。

## 6. 错误

| 代码 | 何时 | 重试 |
| --- | --- | --- |
| `invalid_request` | `messages` 为空、`maxOutputTokens = 0` | 否（调用方缺陷） |
| `model_unavailable` | 模型不可选，或其 Provider、目录缺失 | 用户修正使用者的设置或目录之后 |
| `failed` | 共用策略处理之后仍失败的连接解析、传输、HTTP 或流错误；工具停止 | 由使用者决定 |
| `cancelled` | 调用被丢弃、取消令牌触发，或 Provider 卸载（其停止令牌） | 由使用者决定 |

## 7. 安全

- 凭据不离开 Provider：`ResolvedModelConnection` 在 `complete` 内部构造和使用，从不返回或序列化。
  使用者只看到文本、停止原因和用量。
- 错误消息不含秘密：`failed` 使用现有 `HostError` 的消息，它不包含凭据值；Provider 从不把连接
  格式化进消息。
- 诊断：`complete` 记录一条进程级 `tracing` 事件（target 为 `auxiliary.complete`），含 provider id、model id、
  结果、耗时和 token 用量，从不包含请求或回复内容。使用者在自己的 Session 诊断中记录结果。辅助
  调用不是 Session 事实，不进入任何 Timeline。

## 8. 与现有部件的关系

| 部件 | 关系 |
| --- | --- |
| 默认选择（`provider.select`） | 不变。`complete` 不使用它 |
| SessionTitle（P4） | 调用 `complete(model, …)`；`model` 取其 Settings 来源 `session-title` 中可选的 `model`，否则取前缀中最新 `context/configured` 的模型；该模型是 `external-agent:*` 条目时跳过标题生成。在 `Deps` 中硬依赖 `auxiliaryModel`：没有 Provider（`--models` 配置）时 SessionTitle 报告为失败并指出缺少的服务，而不是悄悄缺席；该配置目前也不装载 TypeScript 插件 |
| 压缩宿主（F1） | 不使用本服务。实现 `ResolveContextModel`；如何选模型、如何读上下文窗口由 F1 设计 |
| Collaboration | S4 不带来变化。成员模型 = profile 的模型，否则为默认选择；如果以后需要单独的成员默认值，那是 Collaboration 自己的设置 |
| Modality | 把每个类别的默认模型保存在自己的 Settings 来源 `modality` 中，并显式传入模型。非对话型连接解析是另一项 Provider 补充，不在 S4 内 |
| Workflow | `stage.model` > profile 模型 > Agents 默认选择已经覆盖模型选择；Provider 中没有阶段默认值 |
| ExternalAgent | 无特殊处理：模型可以是 `external-agent:*` Provider 条目；ModelHost 像其他请求一样路由 |
| `--models` API key 配置（ConfiguredModels，没有 Provider 插件） | 不提供 `auxiliaryModel`；SessionTitle 因此跳过标题生成 |

## 9. 规则检查

| 规则 | 回答 |
| --- | --- |
| 原则 §3（依赖方向） | Provider（核心）只依赖 Aimux（核心）；使用者依赖 Provider；Provider 不点名使用者。 |
| §4 规则 3（before 回调） | 不使用。 |
| §4 规则 8（基础设施不变） | 不改 Timeline、Dispatcher、事件格式、rutis 或装配；使用 `ModelHost`、`ProductResolver` 和现有目录。不向 rutis 提需求。 |
| §4 规则 9（只有需要的事实进 Timeline） | 辅助调用不写事件。 |
| ADR 002 D2（无接线模块） | 不新增插件；服务放在模型与凭据的所有者中。 |
| ADR 002 D3（依赖） | 核心不硬依赖可选插件；SessionTitle（可选）可以依赖本服务。 |
| ADR 002 D4（配置） | Provider 不新增配置；每个使用者的模型选择是它自己的业务配置。 |
| ADR 002 D5（语言中立契约） | §4.1 是契约；§4.2 是一种绑定。 |

## 10. 开放问题

推迟：按用途的推理强度（功能清单："Auxiliary model and reasoning effort"）。如果要加，它是
使用者自己设置里的一个字段，`complete` 把它作为请求参数接收；随 `/settings` 设计决定，不在 S4 内。

## 11. 验收（流程测试）

1. 对 mock HTTP provider 调用 `complete`：请求发往给定模型，带 system 文本、messages、
   `max_tokens` 且无工具；回复是文本加用量。
2. 模型被禁用或不存在：`model_unavailable`，不发请求，不回退。
3. 删除模型所属的 Provider：下一次调用返回 `model_unavailable`。
4. 流进行中丢弃调用会取消请求；卸载 Provider 会以 `cancelled` 结束进行中的调用。
5. 托管账号恢复后的凭据再次被拒：账号被结算（登出），调用以 `failed` 失败；端点恰好收到两次请求。
6. 回复、错误和 trace 事件都不含 API key；调用后 Session 的 Timeline 没有新事件。
