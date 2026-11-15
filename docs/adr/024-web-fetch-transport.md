# 024: Web 抓取层保留自有传输，不换成请求库

Decision status: Proposed.

## Context

`web` 的抓取层自己建连：解析、校验、`CONNECT` 隧道、TLS、连接时限、取消都在
[`web/network.rs`](../design/plugins/Web.md) 里，约 970 行实现。地址策略（A.4）改为可选后，
“必须逐地址校验才能决定怎么连”这条理由不再成立，模块的体量与它提供的两个工具不匹配
（[#1609](https://github.com/arcships/dim/issues/1609)）。

按 `reqwest` 重写这条路走过一遍，没有提交。三处与现行契约冲突：

| 冲突点 | 现状契约 | 换库后 |
| --- | --- | --- |
| `http` 经代理 | A.4 规定 `http` 与 `https` 都发 `CONNECT <目标>:<端口>` | `reqwest` 对 `http` 发 absolute-form 请求行（`GET http://… HTTP/1.1`），只有 `https` 才建隧道；代理侧能看到完整 URL，现有代理夹具和契约测试都按 `CONNECT` 写 |
| 地址固定 | A.4/A15：解析、校验、连接使用同一份地址，检查与连接之间不可能重绑定 | `reqwest` 在解析与连接之间没有钩子。`blockPrivateAddresses` 只能退化为请求前的检查，`allowPrivateHosts` 从“连接层保证”降级为“尽力而为” |
| 错误分类 | Appendix C 区分 `tls_failed` 与 `connect_failed` | `reqwest::Error` 不暴露 TLS 阶段，两者只能合并 |

三处都要改文档、改验收、改夹具。第一处是行为可见的变化（代理侧日志从主机名变成完整 URL），
第二处是安全保证的降级，第三处是错误码表的收缩。这些不是重写的附带品，是需要单独决定的契约。

## Decision

### D1. 传输保留在 Web 内部，不引入请求库

`web_fetch` 的连接路径继续由 `web/network.rs` 拥有：自建 TCP/TLS、自建 `CONNECT`、
固定的连接时限与取消。可读性让位于两件事：代理看到什么，以及地址校验与连接使用同一份解析结果。

这条决定不评价 `hyper` 是否优雅，只记录换库会改变的两个对外事实。

### D2. 代理永远显式，不采用任何库的“默认代理行为”

代理来源按 [Web A.4](../design/plugins/Web.md#a4-address-policy) 的顺序确定：配置的 `proxy`，
否则环境变量，否则桌面设置；`DIM_WEB_SYSTEM_PROXY=off` 关闭后两级。不允许某个库在未记录的
情况下替这个工具决定出口。

### D3. 内容转换不受本决定约束

`content.rs`（HTML → Markdown，约 630 行）可以单独换成 `html2text` 一类的实现，
只要 `convert()` 的输入输出和 A.3 的文本格式不变。这条不需要与本决定的传输问题一起动。

### D4. 重新评估的条件

以下三条中至少两条被明确接受后，才重新提交换库：

1. `http` 经代理改用 absolute-form，并接受代理侧可见完整 URL；
2. `blockPrivateAddresses` 明确降级为请求前检查，`allowPrivateHosts` 的语义随之改写；
3. `tls_failed` 合并进 `connect_failed`，Appendix C 的码表收缩。

## Consequences

- `web/network.rs` 维持现状；A.4 与 A15 的保证不变。
- 抓取层的体量问题只能用 D3 和文档收缩缓解，不能靠换库解决。
- 换库的净收益在动手前被高估了：省下约 700–900 行实现，同时要重写约 300 行代理夹具与
  契约测试，并失去一条可验证的保证。
- 若将来 `reqwest` 暴露“连接前地址钩子”或“强制隧道”，D4 的前提会变，届时新开一条 ADR，
  本条不改。

References: [Web 设计](../design/plugins/Web.md), [#1609](https://github.com/arcships/dim/issues/1609)。
