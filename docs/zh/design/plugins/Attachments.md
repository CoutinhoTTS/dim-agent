# Attachments

状态：草案。总体设计：[overview B.4](../overview.md)。阶段：P1。语言：Rust。作用域：Session（[ADR 002](../../adr/002-plugin-scope.md)）。

让用户在一次提交中附加图片和大段文本。图片不需要插件：CLI 在 `session.submit` 中把它们作为图片内容块发送，由 Rpc 在提交边界校验。Attachments 是一个小型 Session 插件，以 `SessionMethodContribution` 贡献一个 Session 方法 `attachment.put`（[session-boundary](../contracts/session-boundary.md)）。该方法把文本附件（一次大段粘贴）存为 SessionStore 在 Session 资源节点上拥有的不可变 Session 文件，并返回其路径；用户文本引用该路径，模型在需要时用 `read` 读取文件。这些文件在 fork 时的共享与删除属于 SessionStore 的行为。没有事件。

## 1. 结构

```mermaid
flowchart LR
  CLI["CLI / 客户端"]
  subgraph P["进程作用域"]
    RPC["Rpc session.submit（边界校验）"]
    SS["SessionStore Session 文件（资源节点）"]
  end
  subgraph S["Session 作用域"]
    AT["AttachmentsPlugin（SessionMethodContribution attachment）"]:::new
    SI["Session 输入"]
  end
  CLI -- "带图片块的 session.submit" --> RPC
  CLI -- "attachment.put" --> RPC
  RPC -- "解析 Session，分派" --> AT
  AT -- "存储文件" --> SS
  RPC -- "input/message" --> SI
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
```

Attachments 只依赖 SessionStore，一个外层作用域插件（[ADR 002](../../adr/002-plugin-scope.md) D3）；它从不写 Timeline，从不接触图片，也不拥有自己的数据。它只安装在启用了它的 Session 中。

| 部分 | 作用域 | 依赖 | 提供 |
| --- | --- | --- | --- |
| `AttachmentsPlugin` | Session | `S:SessionIdentity`；SessionStore Session 文件补充 | `S:SessionMethodContribution[attachment]`（`attachment.put`，级别 `session`） |

依赖的契约：`SessionMethodContribution`（Rpc 解析 Session 并分派）；SessionStore 的一项补充，在 Session 资源节点上存储不可变文件、在 fork 时共享、并随最后一个拥有它的 Session 删除（overview 附录 C，SessionStore）；Rpc 对 `session.submit` 中图片块的校验（Rpc 补充）。本文定义的契约：`attachment.put`、其错误码与限制、CLI 引用格式。Attachments 不引用任何其他插件的内部类型。

## 2. 数据与状态

```mermaid
flowchart TB
  subgraph SSD["SessionStore：Session 目录（A1）"]
    F["attachments/attachment_id/name：不可变文本"]
  end
  subgraph TL["Session Timeline"]
    IM["input/message：用户内容中的图片块；路径文本"]
  end
  PUT["attachment.put"] --> F
  FORK["SessionStore fork：共享，不复制字节"] --> F
  DEL["SessionStore delete：随最后一个拥有它的 Session 删除"] --> F
  SUB["Rpc session.submit"] --> IM
```

Attachments 没有状态：文件属于 SessionStore 中该 Session 的数据，图片只存在于 Rpc 以用户为作者写入的 `input/message` 中。`attachment_id` 是随机 UUID，因此 fork 继承的历史中的路径保持有效，因为 SessionStore 共享文件而非复制（A6）。

事件：无。图片是现有 `input/message` 的一部分；文本附件只以路径形式出现在用户文本中。

## 3. 流程

### 3.1 提交时的图片

```mermaid
sequenceDiagram
  participant C as CLI
  participant R as Rpc
  participant SA as Session
  participant TL as Timeline
  C->>R: session.submit {user: [image base64, text]}
  R->>R: 边界校验（格式、限制、模型图片能力）
  R->>SA: Session::input(branch, user content)
  SA->>TL: Branch::input -> Timeline::append
  R-->>C: 已接受，或 attachment_* 错误且不写入
```

校验在输入写入之前进行。拒绝时不写入任何内容，CLI 保留草稿。

### 3.2 文本粘贴

```mermaid
sequenceDiagram
  participant C as CLI
  participant R as Rpc
  participant A as Attachments
  participant SS as SessionStore
  C->>R: attachment.put {sessionId, name, text}
  R->>A: SessionMethodContribution 调用（Session 由 Rpc 解析）
  A->>SS: 存储不可变文件（Session 的资源节点）
  A-->>C: {attachmentId, path, bytes, lines}
  C->>R: session.submit，文本中包含路径引用
```

路径只在文件持久化之后返回。模型之后用 `read` 读取它。

## 4. 时序与冲突

| 场景 | 结果 |
| --- | --- |
| 未启用 Attachments | `attachment.put` 得到 `unknown_method`；图片不受影响 |
| 提交后切换到不支持图片的模型 | 图片到达新模型；其适配器拒绝或丢弃它 |
| 图片输入的 `input/withdrawn` | 从不被消费；文本文件保留在 Session 中 |
| Fork 与源的删除竞争 | 由 SessionStore 的 fork/delete 顺序决定；文件共享，从不复制 |
| 资源节点离线 | 图片不受影响；对路径的 `read` 像任何 `read` 一样失败 |

完整表格：附录 B。

## 5. 上下文映射

| 附件 | 模型看到什么 | 位置 |
| --- | --- | --- |
| 图片 | 输入的 user 消息中的 `ContentBlock::Image`（位于其 raw 块之后） | 消费位置（`input/consumed`） |
| 文本附件 | 用户文本中的路径；内容只通过之后的 `read` 结果出现 | user 消息位于消费位置；内容位于该工具结果的位置 |

压缩既不逐字保留图片也不保留文件内容，只保留摘要（若摘要保留了路径，路径得以保留；文件仍在）。子分支继承切点之前已消费的输入；fork 出的 Session 继承固定前缀，共享文件保持可读。插件直接写入的输入不经过 `session.submit` 校验（A8）。模型从不看到附件 id、被拒绝的提交或诊断。

## 6. 接口（摘要）

| 接口 | 类型 | 用途 |
| --- | --- | --- |
| `attachment.put` | `SessionMethodContribution`，级别 `session` | 把文本附件存入 Session 目录，返回其路径 |
| `session.submit` 中的图片块 | Rpc 边界校验（补充） | 校验格式、限制与模型能力 |
| `attachment_*` 错误 | `session.submit` 和 `attachment.put` 上的 `appError` | 拒绝原因 |

## 7. 安全

图片只为校验头部和限制而解码。文本附件由 SessionStore 写在 Session 目录下，从不写到客户端指定的路径（`name` 是文件名）。文件在 Session 的权限策略下可通过 `read` 读取；附件不授予额外权限。远程配对设备可以像调用任何 Session 方法一样调用 `attachment.put` 和 `session.submit`。内容是重放给模型的用户数据；CLI 不得隐式附加机密。fork 只读取其继承历史所引用的文件。

## 附录 A. 接口细节

### A.1 `session.submit` 处的图片校验（Rpc 补充）

| 输入块 | 处理 |
| --- | --- |
| `Text` | 不变 |
| `Image { source: Base64 }` | 解码；按 magic bytes 嗅探格式（忽略声明的 `media_type`）；接受 PNG、JPEG、WebP、GIF（功能列表）；设置 `media_type`；执行限制（A2） |
| `Image { source: Url }` | 拒绝 `attachment_invalid`（A3） |
| 任何图片，已采用的模型不支持图片输入 | 拒绝 `attachment_unsupported_model` |
| 任何图片，能力未知 | 接受；由模型适配器决定（A4） |

限制（A2）：每次提交至多 20 张图片，每张解码后 5 MiB，每边 8000 px；违反者被拒绝，从不缩放。块保持原有顺序。错误以 `invalid_params` 呈现，`appError` 为 `attachment_invalid` / `attachment_too_large` / `attachment_unsupported_model`。

### A.2 `attachment.put`

`attachment.put {sessionId, name?, text}` → `{attachmentId, path, bytes, lines}`。

| 规则 | 内容 |
| --- | --- |
| `name` | `[A-Za-z0-9._-]{1,64}`，不以点开头；默认 `paste.txt` |
| `text` | UTF-8，至多 10 MiB（A2）；空文本被拒绝 |
| 存储 | SessionStore 在其资源节点上 Session 的附件根目录下原子写入 `attachments/<attachmentId>/<name>`（A1）；节点离线时返回 `node_unavailable` |
| `path` | Session 资源节点上的绝对路径，与 `read` 为该 Session 解析的一致 |
| 错误 | `session_closed`、`attachment_invalid`、`attachment_too_large`、`node_unavailable`、`service_reloading`、`unknown_method` |

Rpc 像对任何 Session 方法一样解析 Session。在途调用在 SessionStore 提交之前检查取消。

### A.3 CLI 契约

CLI 读取图片（剪贴板、`@`、粘贴的路径），并在 `session.submit` 中以 base64 图片块发送；超过其阈值的粘贴走 `attachment.put`，并插入 `[Pasted text: <path> (<lines> lines)]`（A5）。被拒绝时保留草稿。临时图片文件在收到提交回复后由 CLI 删除。

## 附录 B. 完整冲突表

| 场景 | 序号顺序 | 结果 | 决定方 |
| --- | --- | --- | --- |
| turn 运行期间提交图片 | 按模式输入 | 像任何输入一样被应用和消费 | InputQueue、Loop |
| 图片输入被消费前 turn 中断 | 已应用 < I | 待处理；由下一个 turn 消费 | Loop |
| 输入的 `input/withdrawn` | 输入 < withdrawn | 从不被消费；文本文件保留在 Session 中 | InputQueue |
| 校验与写入之间分支关闭 | 关闭 < 写入 | `Session::input` 失败 `BranchMissing`；不写入 | Runtime facade |
| 提交后切换模型 | 输入 < `context/configured` < 已消费 | 新模型的适配器拒绝或丢弃图片 | ModelProvider |
| 并发的 Steer/Queue/Append | Timeline 顺序 | 不受影响 | InputQueue |
| 已 `attachment.put`，从未提交 | — | 文件保留在 Session 中；随 Session 删除 | SessionStore |
| `attachment.put` 与 Session 删除竞争 | — | `session_closed`，或文件被删除操作移除 | SessionStore |
| Fork 与源的删除竞争 | — | 按 SessionStore 的 fork/delete 顺序 | SessionStore |
| 压缩 | 已消费 < 压缩 | 只通过摘要保留；文件仍在 | ContextPreparation |
| 子分支切出 | 已消费 < 切点 | 子分支继承图片和路径；同一批文件 | 历史投影 |
| Attachments 重载 | — | `attachment.put` 得到 `service_reloading`；提交前的在途调用以 `operation_interrupted` 结束 | Rpc |
| 提交期间 Rpc 重载 | — | `service_reloading`；客户端检查 `session.input_lookup`，然后以相同的 `submissionId` 重试 | Rpc、客户端 |
| 进程重启 | — | 不丢失状态 | — |
| 重复或迟到的 `attachment.put` | — | 每次调用产生新附件（A7） | Attachments |
| 远程配对设备提交 | — | 相同的边界校验 | Rpc |
| Session 卸载/恢复、客户端 detach | — | 无影响 | — |

## 附录 C. 错误与恢复

| 代码 | 含义 |
| --- | --- |
| `attachment_invalid` | 不支持或无法解码的图片、URL 来源、错误的名称、空文本 |
| `attachment_too_large` | 超过 A.1/A.2 的某项限制 |
| `attachment_unsupported_model` | 向不支持图片输入的模型发送图片 |
| `session_closed`、`service_reloading` | 与其他 Session 方法相同 |
| `node_unavailable` | Session 的资源节点不可达 |
| `unknown_method` | 该 Session 未启用 Attachments |

被拒绝的提交不留下任何事实。存储后没有后续提交的文件随其 Session 删除。任何变更都不会自动重试。

## 附录 D. 验收

1. PNG/JPEG/WebP/GIF 被接受并带有嗅探出的媒体类型；标错类型的 JPEG 被纠正；BMP 和 URL 被拒绝；拒绝时不写入。
2. 非视觉模型：`attachment_unsupported_model`；能力未知：接受。
3. 限制：21 张图片、6 MiB 图片、9000 px 边长被拒绝。
4. `attachment.put` 返回 `read` 能为该 Session 解析的路径。
5. Fork 后删除源：fork 仍能读取文件；删除 fork：文件被移除；不复制字节。
6. 未启用 Attachments：`attachment.put` 得到 `unknown_method`；图片提交不受影响。
7. 通过公开 RPC 覆盖中断、撤回和分支关闭各行。

## 附录 E. 假设

| # | 假设 | 依据 | 若被否决的影响 |
| --- | --- | --- | --- |
| A1 | 文本附件是 SessionStore 拥有的 Session 文件，存储在 Session 资源节点上每个 Session 的附件根目录下；SessionStore 在控制节点上保存所有权记录。对于位于控制节点上的 Session，这就是 Session 目录 | 判断；`read` 在资源节点上运行 | 其他布局，或内联文本内容 |
| A2 | 限制：20 张图片、5 MiB、8000 px、10 MiB 文本；拒绝而非缩放 | 边界情况参考；判断 | 不同的限制或缩放 |
| A3 | 拒绝 URL 图片来源 | Overview "最终内容"；判断 | 由 provider 抓取 URL |
| A4 | 模型能力未知时接受图片 | 判断 | 未知即拒绝 |
| A5 | CLI 决定粘贴阈值与引用格式 | Overview 附录 E（CLI）；判断 | 服务端返回格式化的引用 |
| A6 | Fork 共享不可变文件（SessionStore 行为） | 维护者决定；在新路径复制字节会破坏继承的路径 | 字节复制加路径改写，对固定的历史不可行 |
| A7 | 不去重 | 判断 | 内容寻址的 id |
| A8 | 只有 `session.submit` 校验图片 | 判断 | 其他写入方自行校验其图片 |
| A9 | fork 保持源的资源节点；fork 到其他节点不在范围内 | 维护者决定 | 文件必须在其他节点上可用 |

## 附录 F. 待对齐问题

| # | 问题 | 说明 |
| --- | --- | --- |
| F1 | 边界处的图片能力 | Rpc 需要 Session 已采用模型的图片能力（已发布的 `context/configured` 加上 Provider 目录）；没有它，每张图片都被接受（A4） |
