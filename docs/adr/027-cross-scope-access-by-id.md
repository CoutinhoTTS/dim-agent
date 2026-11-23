# 027: 跨作用域访问按 id 直达所有者

Decision status: Proposed.

## Context

[ADR 002](002-plugin-scope.md) D2 规定，进程插件经 Agents 的 `sessions` 服务访问其他 Session；[Session 访问契约](../design/contracts/session-access.md)据此定义了 `launch`、`input`、`lookup`、`history`、`pending`、`withdraw`、`stop`、`idle`、`delete`。D3 又规定插件只依赖本作用域或外层作用域，所以进程插件不能注入 Session 组或分支组的服务。

这些方法的数据和操作不属于 Agents：输入与撤回属于 InputQueue，历史属于 Timeline 的读取所有者，停止属于 Loop。Agents 每个方法都要先取得 Session，再转述对方的结果、错误和新鲜度。新增一种跨 Session 的需求，就要在 `sessions` 上再加一个方法。Session 部件访问本 Session 内分支所有者时，也有同样的问题。

[设计规则 §7](../architecture/rust-agent/principles.md#7-所有者与操作) 要求调用方直接请求数据的所有者，不代查，也不设只转发的门面。要满足这条，调用方需要一条能从外层作用域直接到达内层所有者的路径。

## Decision

### D1. 外层作用域的调用方按 id 直达内层所有者

- 进程作用域的调用方用 Session id 到达 Session 组的服务，用 `(Session id, line id)` 到达分支组的服务。
- Session 作用域的调用方用 line id 到达本 Session 分支组的服务。
- 解析由机制完成，调用方直接调用目标所有者自己的操作。
- 中间的所有者不按操作名转发，不包一层请求或回复，也不转述目标所有者的错误。
- 目标不存在、未加载或正在关闭时，解析本身以错误结束，错误种类由机制定义。

### D2. 这是按 id 调用，不是依赖

- D3 的依赖方向不变：调用方不把内层服务声明为依赖，也不参与内层的装配顺序。
- 调用方声明要调用的服务名，每次调用时按 id 解析。
- 解析结果不跨调用保存，也不交给别的部件。

### D3. 让目标保持加载属于目标作用域的所有者

- 进程调用方需要一个 Session 在多次调用之间保持加载时，向 Agents 取一个绑定调用行的保持。行卸载时，保持随之结算。
- 保持只决定 Session 是否加载，不提供对 Session 内部的访问。

### D4. Agents 只保留自己的操作

- Agents 拥有 Agent 与 Session 的实例：创建、关闭、删除，准入值，以及保持。
- `sessions` 服务中属于其他所有者的方法删除，调用方按 D1 直接调用这些所有者：
  - `input`、`lookup`、`pending`、`withdraw` 调 InputQueue；
  - `history` 调 Timeline 的读取所有者；
  - `stop` 调 Loop。
- 各方法的去向由 Agents 的契约给出。

## Consequences

- ADR 002 D2 表中「Reach other Sessions」一行由 D1 取代。
- Session 访问契约的 `sessions` 服务按 D4 收缩，契约由它的所有者修改。
- Collaboration、Scheduler、Workflow、EventHub、Rpc 的跨 Session 调用都改成 D1 的形状。
- 机制要提供按 id 解析 Session 组与分支组服务的能力，而且它跨语言可用。

## Non-goals

- 不改变作用域划分（ADR 002 D1）和依赖方向（D3）。
- 不改变机制登记点：`sessionMethods`、`managementMethods`、`toolRegistry`。
- 不规定任何语言的写法，写法属于绑定。
- 不改变客户端协议：客户端仍经 Rpc 的方法表访问 Session。

## Supersession

取代 [ADR 002](002-plugin-scope.md) D2 表中「Reach other Sessions」一行。D2 的其余各行和 D1、D3–D5 不变。
