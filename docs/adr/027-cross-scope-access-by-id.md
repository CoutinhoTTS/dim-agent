# 027: 跨作用域访问按 id 直达所有者

Decision status: Proposed.

## Context

[ADR 002](002-plugin-scope.md) D2 规定，进程插件经 Agents 的 `sessions` 服务访问其他 Session。[Session 访问契约](../design/contracts/session-access.md)据此定义了 `sessions` 的方法。ADR 002 D3 又规定插件只依赖本作用域或外层作用域，所以进程插件不能注入 Session 组或分支组的服务。

`sessions` 的方法分两类：
- `launch`、`exists`、`delete` 是 Agents 自己的操作；
- 其余方法的数据和操作属于 Session 或分支内的所有者，例如输入的撤回与待处理属于 InputQueue，默认分支属于 TimelineTree，停止属于 Loop。

对第二类方法，Agents 要先取得 Session，再转述对方的结果、错误和新鲜度。每新增一种跨 Session 的需求，就要在 `sessions` 上加一个方法。Session 部件访问本 Session 内的分支所有者，也有同样的问题。

[设计规则 §7](../architecture/rust-agent/principles.md#7-所有者与操作) 要求调用方直接请求数据的所有者，不代查，也不设只转发的门面。这需要一条从外层作用域到内层所有者的路径。rutis 已经按实例键提供数据面的派发：每个 Session 实例和分支实例下，都有该作用域数据面的派发入口（[Dim 如何使用 rutis](../design/rutis-usage.md)）。

## Decision

### D1. 外层作用域的调用方按 id 直达内层所有者的数据面

- 进程作用域的调用方：用 `sessionId` 到达 Session 组的数据面，用 `(sessionId, branchId)` 到达分支组的数据面。
- Session 作用域的调用方：用 `branchId` 到达本 Session 分支组的数据面。
- 机制按实例键解析到目标实例的派发入口，调用方直接调用目标所有者自己的操作。
- 中间的所有者不按操作名转发，不包装请求或回复，也不转述目标所有者的错误。
- 目标不存在、未加载或正在关闭时，解析以错误结束，错误种类由机制定义。

### D2. 可解析的只有目标作用域的数据面，不含登记点

- 可解析的是目标作用域在 catalog 中共享的数据面。
- 不包括机制登记点：`sessionMethods`、`toolRegistry` 等登记只能在本作用域内进行，因为登记属于登记方自己的 loader row。
- 也不包括作用域的生命周期服务。
- 各语言使用同一条路径：Rust 插件在进程内调用；TypeScript、Python 等语言运行时和 leaf 经 rutis 会话调用，与其他数据面相同。它们是宿主运行的可信本地代码，不靠 id 隔离（[Dim 如何使用 rutis](../design/rutis-usage.md)，AGENTS.md 规则 11）。
- 哪些 link 能到达这条路径由部署决定（[服务面](../design/service-faces.md) §2.1）。客户端不经这条路径，仍经 Rpc 的方法表与 Session 授权访问。

### D3. 按 id 调用不是依赖

- ADR 002 D3 的依赖方向不变：调用方不把内层服务声明为依赖，也不参与内层的装配顺序。
- 调用方声明要调用的服务名，每次调用时按 id 解析。
- 解析结果不跨调用保存，也不交给别的部件。

### D4. 让目标保持加载属于 Agents

- 进程调用方需要一个 Session 在多次调用之间保持加载时，向 Agents 取一个保持。
- 保持属于调用方的 loader row，在调用方释放或该 row 卸载时结算。
- 保持只决定 Session 是否加载，不提供对 Session 内部的访问。

### D5. Agents 只保留自己的操作

- Agents 拥有 Agent 与 Session 的实例：创建、关闭、删除，准入值，以及保持。
- `sessions` 中不属于 Agents 的方法删除，调用方按 D1 直接调用目标所有者。
- 每个方法的去向由 [Agents 契约](https://github.com/arcships/dim/pull/2098)给出，目标操作由各所有者的契约定义。

## Consequences

- ADR 002 D2 表中「Reach other Sessions」一行由 D1 取代。
- [Session 边界契约](../design/contracts/session-boundary.md)附录 F 中「No service lookup by name is added to the Session facade」不变：D1 的路径是机制的派发，不在 Session facade 上加查找。
- Session 访问契约中 `sessions` 的方法表由 Agents 契约取代，Session 访问契约随 Agents 的契约改动一起修改。
- Collaboration、Scheduler、Workflow、EventHub、Rpc 的跨 Session 调用都改成 D1 的形状。
- 机制要按实例键解析 Session 组与分支组的数据面，并且经 rutis 会话对各语言运行时和 leaf 可用。
- Session 访问契约 §7「never exported over RPC entries or rutis links」随 `sessions` 一起取代：D1 的路径对语言运行时和 leaf 可用，对 RPC 入口不可用。

## Non-goals

- 不改变作用域划分（ADR 002 D1）和依赖方向（ADR 002 D3）。
- 不改变机制登记点及其登记方式。
- 不规定任何语言的写法，写法属于绑定。
- 不改变客户端协议：客户端仍经 Rpc 的方法表访问 Session。

## Supersession

取代 [ADR 002](002-plugin-scope.md) D2 表中「Reach other Sessions」一行。D2 的其余各行和 D1、D3–D5 不变。
