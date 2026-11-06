# 模型失败

一次失败的模型请求会报告什么，以及重试策略如何读取它（issue #1197）。

## 1. 一个封闭的 kind，在读取提供方错误的地方决定

aimux 桥接层是唯一持有提供方结构化错误的地方，因此由它决定 `ModelFailureKind`。它只使用 origin、HTTP 状态、归一化后的 code 和 `AiMuxError` 变体，从不用消息文本做匹配。

| kind | 依据 | 是否重试 | 含义 | 动作 |
| --- | --- | --- | --- | --- |
| `network` | `ApiCall`，origin 为 Transport，无 HTTP 状态 | 是 | 无法连接提供方，或连接中断 | 检查网络或代理 |
| `rate_limited` | 429、限流 code | 是；`Retry-After` 在等待预算内时用作延迟 | 提供方正在限流 | 等待后重试 |
| `server_error` | 408、除 501 和 505 外的 5xx、server 和 overload code、可重试的流事件 code | 是 | 提供方报告服务器错误 | 稍后重试或换一个模型 |
| `malformed_response` | `InvalidResponseData`、`JsonParse`、`InvalidToolInput`、`NoSuchTool`、`TypeValidation`、无法解码的流 | 否 | 提供方发来的响应 Dim 无法使用 | 重试；若反复出现，换一个模型或检查代理或网关 |
| `auth_rejected` | 401、403、无效 key/认证 code | 否（保留现有的一次凭据恢复） | 提供方拒绝了凭据 | 重新登录或检查 API key |
| `quota_exhausted` | 402、配额/计费 code | 否 | 账户没有配额了 | 检查套餐或计费 |
| `not_found` | 404 | 否 | 提供方找不到模型或端点 | 检查模型和提供方 base URL |
| `context_too_long` | 413、上下文长度 code | 否 | 对话超过模型可接受的长度 | 压缩对话或新开会话 |
| `content_filtered` | 内容策略 code | 否 | 提供方的内容过滤拦截了响应 | 改写请求 |
| `invalid_request` | 400、409、422、501、505、其他 4xx、invalid-request code（含 `max_tokens_exceeded`）、`InvalidArgument`、`InvalidPrompt`、`UnsupportedFunctionality` | 否 | 提供方拒绝了请求 | 查看详情 |
| `output_limit` | `max_tokens` 停止 | 否 | 模型达到输出上限 | 提高输出 token 上限后重试 |
| `unknown` | 其他 | 否 | 模型请求失败 | 查看详情 |

HTTP 连接或流的超时以传输 `ApiCall` 到达桥接层，是 `network`。`AiMuxError::Timeout` 在聊天调用中没有产生者；若它到达桥接层，是 `unknown`，且不重试。

code 比较前先 trim 并转 ASCII 小写。

- 409、501、505 无论随带什么可重试 code 都是 `invalid_request`；永久 code 仍然决定自己的 kind。
- 流事件失败携带的是响应的状态而不是事件的状态，因此只由 code 决定：可重试 code 映射为 `rate_limited` 或 `server_error`，永久 code 映射到其 kind，其余为 `unknown`。

桥接层产生的每个 `HostError::Failed` 都带事实，包括 `AiMuxError::Timeout`、`InvalidResponseData`/`JsonParse`、`InvalidArgument`/`InvalidPrompt` 和 `UnsupportedFunctionality`。

kind 的好坏取决于提供方的结构化字段。若提供方把配额错误报成一个只有 message 的裸 429，它就会成为 `rate_limited` 并在预算内被无用地重试；`reason` 仍会显示提供方自己的消息。缺失的 code 加到桥接层的 code 列表里，绝不靠文本匹配。

## 2. 事实

`ModelOutputOutcome::Failed` 新增 `failure: Option<ModelFailure>`：

```
ModelFailure {
  kind,             // 第 1 节
  status?,          // 收到响应时的 HTTP 状态
  providerCode?,
  requestId?,
  reason?,          // 一条具体句子，单行，最多 300 字符
  cause: string[],  // 由外到内的链；每帧单行，最多 200 字符，最多 6 帧
  attempts,         // 至少 1
  waitedMs,         // 退避等待总时长
  retryAfterMs?     // 提供方要求的等待超过剩余预算，因此停止
}
```

- `failure` 可选。旧事件里不存在；不属于模型提供方调用的失败也没有：agent loop 在重载时对未结算调用的失败（`agent_loop.rs`）和上下文准备失败（`context_preparation.rs`）。字段的大小写和 `optional_non_null` 处理与 `presentation` 一致。
- `reason`：响应或流事件失败取提供方的错误消息；传输失败取原因链最内层。JSON 解析的消息内嵌了它读不懂的响应体文本，因此其 `reason` 取错误结构中的解析诊断；错误没有 cause 时为空。工具失败从 `tool_name` 字段命名：`invalid arguments for tool '<tool_name>'` 或 `the model called an unknown tool '<tool_name>'`。`TypeValidation` 不带 reason，因为它的消息内嵌了被校验的值。工具输入、可用工具列表和被校验的值从不存储。
- `cause`：来自 `ApiCallError.message` 和 `ErrorCause` 的链，由外到内，每帧一行。`providerCode` 和 `requestId` 是单行且有界，最多 200 字符；为空则不存在。
- 从不存储：请求 URL、请求头、请求值、响应体或任何凭据。aimux 已丢弃 URL；桥接层不得复制 `url`、`request_body_values` 或 `response_body`，`reason` 和原因帧只是去除了控制字符的文本。
- 旧客户端读取的 `error` 是默认英文行 `<meaning>: <reason>. <action>`，没有 reason 时省略 `: <reason>`。不再有 `ApiCall:` 前缀。该失败结束的回合在 `TurnOutcome::Failed.error` 中携带同一字符串，客户端据此可以区分两者并去掉重复。
- `max_tokens` 停止保留其 presentation，同时写入 `failure.kind = output_limit`。
- 没有提供方边界的宿主机失败得到 kind `unknown` 和它原本携带的消息。

## 3. 谁来措辞

客户端从 `kind` 出发，经各自的 i18n 组织自己的那一行。事实携带 kind 和数据，而不是句子。`presentation` 保持原意：宿主机或 `ModelFailurePolicy` 提供的措辞优先于客户端的 kind 表。它是覆盖项，不是始终存在的形式。

终端事实的 `error` 是一条完整的兜底行，因此所有已经打印 `error` 的渲染器无需自身改动就能显示更好的行。

## 4. 重试

ModelProvider 重试时：
- 最多 3 次尝试；
- 退避为 0.5 秒 × 次数 × [0.75, 1.0] 的抖动；
- 在 60 秒等待预算内；
- `Retry-After` 在剩余预算内时用作延迟，放不下时立即停止并记录 `retryAfterMs`；
- 只重试 `network`、`rate_limited` 和 `server_error` 三种 kind，因此除 501 和 505 外的 5xx 状态现在会被重试。

已经输出不可重放内容后不重试。中断会取消等待。凭据拒绝保留一次恢复，且没有延迟。

## 5. 兼容性

字段是增量的。没有 `failure` 的二进制无法解码带它的事件，因为 `ModelOutputOutcome` 拒绝未知字段。这与 `presentation` 的前向兼容限制相同，因此跨此变更的降级不受支持，正如它此前对 `presentation` 也不受支持。
