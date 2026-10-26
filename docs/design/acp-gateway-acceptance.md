# ACP Gateway acceptance

验收对象是 [AcpGateway 合同](acp-gateway-implementation.md) 及 [TemporaryMcp](plugins/TemporaryMcp.md) 中明确支持的子集。Agent core、RPC 框架、内置 Session 协议和持久化格式没有改动；MCP 业务目录注册独立临时插件并复用现有服务器实现，持久 MCP 管理保持原行为。Draft PR：[arcships/dim#441](https://github.com/arcships/dim/pull/441)，主需求：[#436](https://github.com/arcships/dim/issues/436)。

## Build and environment

| 项目 | 验收值 |
| --- | --- |
| 日期 / 平台 | 2026-10-26，macOS arm64 |
| 实现源码 commit | `0c3f12d75f0c62af45d1cb9f6ee6f248d535570b` |
| Native executable | `dimcode 0.1.0-alpha.6`，本仓库 `cargo build --locked -p dimcode`，Mach-O arm64 |
| Executable SHA-256 | `e449baef8ec14ba108d767a2bbb841b7b440b79664b8b1f6193296c1cfb4efe1` |
| Cargo.lock SHA-256 | `41ce54e8ffcddcfb2d9c7994be37779b538d1fc37b3a7a9c07ba9def0a366fd5` |
| ACP | protocol v1；Rust SDK 3.5.0，schema 1.12.0；源协议及 Lody Core 固定版本见合同 §2 |
| acpx | 0.7.0 |
| Lody | Nightly 0.109.0-nightly.0，build 1.1.39 |
| 模型 / 账户 | loopback OpenAI-compatible fixture，无真实凭据；managed-provider flow 使用公开 RPC fixture |
| 数据 | 独立临时 home 和 workspace；不使用产品 home。acpx 使用唯一临时 cwd，但仍按其自身实现写用户 `.acpx` 记录 |

## Automated public flows

| 范围 | 结果 / 证据 |
| --- | --- |
| CLI compatibility | `cli_flow -- --skip acp_stdio_recovery_deadline`：120 passed，11 ignored，1 filtered；包括当前 CLI 的旧 daemon 替换行为。本轮没有复跑被过滤的恢复截止用例 |
| ACP stdio | 回归覆盖现有 ACP 和 terminal-auth helper；两个临时 MCP stdio flows（含 Node opt-in）另行执行通过 |
| 临时 MCP | native MCP Session suite 13 passed，含真实 30 秒租约过期、失败清理、Session 隔离、无设置写入和真实工具调用；工具列表变更另行复核通过 |
| 既有 MCP | `mcp_flow` 5 passed；`mcp_http_flow` 7 passed、4 ignored。原有配置、连接代际、HTTP/SSE、取消和关闭行为回归通过 |
| 共享传输 | `dim-client` 两个公共连接/callback 测试通过；身份/profile 校验、交错响应、半帧取消和受限 loopback callback |
| Browser authentication | `dim-acp` 三个公共宿主 flow 通过：真实 callback listener/native complete、请求取消、EOF 时先取消自己的 pending attempt 再释放 RPC |
| 静态检查 | 受影响 crate 的 all-target Clippy、格式和 whitespace 检查通过 |
| 架构 | xtask source/crate boundaries、20 个 plugin documents、service catalog 与 service declarations 通过；357 modules / 2122 edges；graph 由 xtask 生成 |

流程使用真实 `dimcode acp` stdio 和公开 RPC socket，而不是替代 App Server trait。production flow 连接本次构建的 App Server 和 loopback HTTP 模型，得到 `ACP_REAL_OK`。

复现入口：

```sh
cargo test --locked -p dimcode --test cli_flow
cargo test --locked -p dim-client -p dim-acp
cargo test --locked -p dim-app-server --test mcp_session_flow --test mcp_flow --test mcp_http_flow
cargo test --locked -p dimcode --test cli_flow acp_stdio_temporary_mcp -- --include-ignored
cargo clippy --locked -p dim-app-server -p dim-acp -p dim-cli --all-targets -- -D warnings
cargo run --locked -p xtask -- check
```

恢复截止 flow 和 native MCP 过期 flow 各故意等待真实 30 秒。前者可用 `cargo test --locked -p dimcode --test cli_flow acp_stdio_recovery_deadline` 单独运行；本轮执行的是后者。Node MCP flow 需要已安装的 runtime MCP SDK，复用既有官方 SDK fixture。测试使用自己的隔离 home。

覆盖的行为包括：

- initialize、new/list/load/resume、prompt、close；固定头空页、load/load 和 load/resume 共享准备；标题元数据刷新；resume 不重放旧消息。
- submission reply 与 consumed/turn-end 的竞态、另一 turn 提前结束、Unicode scalar progress/committed 去重；撤回持有输入、无本地 prompt 的 addressed stop、stop/close 的顺序。
- read request 取消不结算其他 waiter；迟到准备不能覆盖关闭后新建的准备；128 个在途请求准入与结算后的释放；慢客户端可靠输出溢出明确失败，原生端只观察到 detach。
- 临时 MCP 首次请求即可见、模型实际调用及工具结果回到模型；headers/environment/cwd 接线；工具列表变更；同 bindingId 幂等、不同配置冲突；过期后重新建立资源且不重发输入；忙碌请求不改变绑定。
- Gateway EOF 结算正在等待的 stdio MCP 调用，官方 SDK fixture 收到取消且子进程退出。native 失败准备撤销资源，30 秒未续租后关闭连接；Session close 同时释放既有和临时连接，持久 MCP 文件保持不变。
- all-branch approval、form 多选/custom、图片和文本 resource；mode CAS 保留工具列表，冲突刷新且不重试；model 使用 catalog 的真实 provider 映射及采用结果。
- auth discovery、已登录不重复 begin、terminal method 不走 authenticate；helper 用同一 home 重新发现 method，并复用 CLI device flow。
- RPC 断线和 daemon instance 变化；丢失 submit reply 后 lookup 同一 submissionId，重建真实终态；丢失 create reply 后不补建 Session。重连、lookup 和 catch-up 共用 30 秒恢复截止点；排队 RPC 也按连接 generation 隔离。恢复 lookup 不回复时，到期先送达 cancelled + nativeCancellation:unconfirmed，再以连接不可用退出；不宣称 native stop 已执行。
- Lody history、真实 terminal turn fork、startup model 优先级、非 new 覆盖拒绝；后台 task 与父工具分开；compaction cancelled；同 member 的两次委派得到不同 runId。无 capability / version 不匹配时无自定义 subagent 通知；观察中断产生 unknown/outputIncomplete，新 activation 使用新 runId。

## Local clients

acpx 用本次构建的绝对 executable 路径、隔离 home/model config 和临时 cwd，执行 `--deny-all --format json --timeout 30 exec`。观察到：

```text
initialize -> protocolVersion: 1
session/new -> native sessionId
session/prompt -> agent_message_chunk: ACP_ACCEPTANCE_OK
               -> stopReason: end_turn
EOF -> local cleanup and native detach
```

**本构建的 Lody GUI MCP 工具验收未完成。** Lody Nightly 当前显示登录页，没有可用会话，未继续账号操作。不能把公开协议测试或文本聊天当作本构建的 Lody MCP 工具调用通过。

本构建的真实 MCP 调用证据来自公开 stdio → production daemon → 官方 SDK MCP fixture → model 流程；HTTP 及列表变更来自公开 Session 流程。new/load/resume/fork 在支持接口的 daemon 上使用临时绑定，旧 daemon 缺接口时才接受并忽略列表。客户端配置不写入共享 home，非空 additionalDirectories 仍拒绝。

## Limits and unverified environments

- 私有 OAuth 插件的真实账户、浏览器授权与令牌交换未实机验收；公开 managed-provider RPC、callback 和取消路径已有测试。私有 realm/token 实现不进入本仓库。
- Windows/Linux 实机、npm 发布包、其余环境依赖 ignored flows 和被过滤的恢复截止用例未在本轮验收。本次没有 release、publish 或合入 main。
- 本次运行受影响范围的测试和 Clippy；workspace 全量 Clippy/test 及 CI 的 ignored profiles 没有在本机运行，仍须按 AGENTS.md 在合入前完成。
- Lody GUI 的图片、审批、form、model/mode 和扩展展示未实机验收；对应 Gateway 协议流由公开 stdio 测试验证。
- `providers/*`、Goal、steer/handoff、Lody 精确 token usage、quota、subagent 查询/输出/取消、scheduled tasks、client terminal/fs backend 和 MCP-over-ACP 等维持合同中的不支持/延期状态。临时 MCP 只承接服务器及工具，不新增 OAuth 登录或持久凭据管理。

## Ablation review

保留一个 client-host 插件、一份共享 RPC、一个 native reader 和一个 SDK writer。没有 daemon 接线插件、第二个 Session store、扩展 registry、业务账本或常驻账户 poller。连接 generation、准备 ticket、prompt 输入关联、取消 waiter、输出预算和 task tracker 各自保护已列出的公开流程；不将运行时 handle 作为持久身份。EOF/卸载只结算本地资源及自己的 pending login，并尽力 detach；显式 close/delete 才触发原生业务关闭。

MCP 只增加实际拥有资源的 Session 部件和绑定子插件，复用原有 McpServer、工具定义/执行和 rutis 子树清理。绑定表用于幂等及隔离，租约用于没有 RPC disconnect hook 时回收强杀留下的资源；Gateway 保留配置只为续租和重连，不另存业务状态。失败准备的所有权标记阻止未完成请求留下配置，持久 MCP 目录不变。没有第二套 MCP transport、loader、全局服务目录或持久租约。

可靠业务输出按序列化大小预留至 writer flush，预算 16 MiB；SDK 自行生成的协议错误不计入此预算。单 frame 16 MiB，单 frame 五秒内未 flush 则结束连接。恢复失败先生成已知请求的终态并限时排空输出；公开恢复截止用例验证响应在退出前送达。CLI 在插件自有工作已回收后，不等待被对端堵住的 OS stdio blocking worker，避免退出挂住。该宿主退出处理不改变共享 daemon 的执行生命周期。
