# ACP Gateway acceptance

验收对象是 [AcpGateway 首版合同](acp-gateway-implementation.md) 中明确支持的子集。Agent core、App Server、既有业务插件和持久化格式没有改动。Draft PR：[arcships/dim#441](https://github.com/arcships/dim/pull/441)，主需求：[#436](https://github.com/arcships/dim/issues/436)。

## Build and environment

| 项目 | 验收值 |
| --- | --- |
| 日期 / 平台 | 2026-10-26，macOS arm64 |
| 实现源码 commit | `7b5f2693e67e0dc0ffade6a174f9903a0f164b4b` |
| Native executable | `dimcode 0.1.0-alpha.5`，本仓库 `cargo build --locked -p dimcode`，Mach-O arm64 |
| Executable SHA-256 | `fd97c07e29178472aadc9a01571808266b5d83a6812c31e561521f847737c5cd` |
| Cargo.lock SHA-256 | `70db7931ea58e9ffd0e90dcf4c3062923620dcd4a1e1a3cc4dde84de3042b811` |
| ACP | protocol v1；Rust SDK 3.5.0，schema 1.12.0；源协议及 Lody Core 固定版本见合同 §2 |
| acpx | 0.7.0 |
| Lody | Nightly 0.109.0-nightly.0，build 1.1.39 |
| 模型 / 账户 | loopback OpenAI-compatible fixture，无真实凭据；managed-provider flow 使用公开 RPC fixture |
| 数据 | 独立临时 home 和 workspace；不使用产品 home。acpx 使用唯一临时 cwd，但仍按其自身实现写用户 `.acpx` 记录 |

## Automated public flows

| 范围 | 结果 / 证据 |
| --- | --- |
| CLI compatibility | 全量 CLI 回归 110 passed，10 个既有环境依赖用例 ignored；补充的恢复截止测试独立通过。共 111 个用例有通过证据，不宣称一次全量 111 passed |
| ACP stdio | 15 个 `acp_stdio` flows 和一个真实 CLI terminal-auth helper flow 通过；另一个真实 30 秒恢复截止 flow 通过，最终慢输出诊断复核通过 |
| 客户端 MCP 兼容 | 三个受影响的公开 flow 通过，覆盖 new/load/resume/fork 接受 HTTP/stdio MCP 配置而不启动或转发，且原生 prompt 正常结束；非空 additionalDirectories 仍明确拒绝 |
| 共享传输 | `dim-client` 两个公共连接/callback 测试通过；身份/profile 校验、交错响应、半帧取消和受限 loopback callback |
| Browser authentication | `dim-acp` 三个公共宿主 flow 通过：真实 callback listener/native complete、请求取消、EOF 时先取消自己的 pending attempt 再释放 RPC |
| 静态检查 | 受影响 crate 的 all-target Clippy、格式和 whitespace 检查通过 |
| 架构 | xtask source/crate boundaries、20 个 plugin documents、service catalog 通过；353 modules / 2053 edges；graph 由 xtask 生成 |

流程使用真实 `dimcode acp` stdio 和公开 RPC socket，而不是替代 App Server trait。production flow 连接本次构建的 App Server 和 loopback HTTP 模型，得到 `ACP_REAL_OK`。

复现入口：

```sh
cargo test --locked -p dimcode --test cli_flow
cargo test --locked -p dim-client -p dim-acp
cargo clippy --locked -p dim-client -p dim-acp -p dim-cli --all-targets -- -D warnings
cargo run --locked -p xtask -- check
```

恢复截止 flow 故意等待真实 30 秒，可用 `cargo test --locked -p dimcode --test cli_flow acp_stdio_recovery_deadline` 单独运行。测试由 fixture 创建自己的隔离 home。

覆盖的行为包括：

- initialize、new/list/load/resume、prompt、close；固定头空页、load/load 和 load/resume 共享准备；标题元数据刷新；resume 不重放旧消息。
- submission reply 与 consumed/turn-end 的竞态、另一 turn 提前结束、Unicode scalar progress/committed 去重；撤回持有输入、无本地 prompt 的 addressed stop、stop/close 的顺序。
- read request 取消不结算其他 waiter；迟到准备不能覆盖关闭后新建的准备；128 个在途请求准入与结算后的释放；慢客户端可靠输出溢出明确失败，原生端只观察到 detach。
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

Lody 自定义命令使用相同 native executable 和隔离配置。能力探测完成 `initialize → session/new (mcpServers:[]) → session/close`，UI 显示“就绪”。真实聊天的 `session/new` 携带一个名为 `lody` 的 MCP server；Gateway 忽略该配置，成功返回 native Session，随后 prompt 正常结束，Lody UI 显示 `ACP_ACCEPTANCE_OK` 并恢复可发送状态。

协议摘要只保留 method、字段名、cwd、MCP 数量/名称和能力标志，不保存 prompt 正文、MCP 参数、环境变量、授权 URL 或 token。该验收证明基本聊天可用，不表示注入的 MCP 工具已接入。所有 Session 建立入口均接受并忽略 mcpServers，不启动、连接、转发或持久化这些服务器，会话继续使用原生工具。客户端 MCP 的实际接入另按合同 §12 评估；core/App Server 不变。

## Limits and unverified environments

- 私有 OAuth 插件的真实账户、浏览器授权与令牌交换未实机验收；公开 managed-provider RPC、callback 和取消路径已有测试。私有 realm/token 实现不进入本仓库。
- Windows/Linux 实机、npm 发布包和 10 个需要额外环境的既有 ignored flows 未验收。本次没有执行 release、publish 或 merge。
- 本次运行受影响范围的测试和 Clippy；workspace 全量 Clippy/test 及 CI 的 ignored profiles 没有在本机运行，仍须按 AGENTS.md 在合入前完成。
- Lody GUI 的图片、审批、form、model/mode 和扩展展示未实机验收；对应 Gateway 协议流由公开 stdio 测试验证。
- `providers/*`、Goal、steer/handoff、Lody 精确 token usage、quota、subagent 查询/输出/取消、scheduled tasks、客户端 MCP/terminal/fs backend 等维持合同中的不支持/延期状态。不能以 native 存在某个入口就声明整个扩展已支持。

## Ablation review

保留一个 client-host 插件、一份共享 RPC、一个 native reader 和一个 SDK writer。没有 daemon 接线插件、第二个 Session store、扩展 registry、业务账本或常驻账户 poller。连接 generation、准备 ticket、prompt 输入关联、取消 waiter、输出预算和 task tracker 各自保护已列出的公开流程；不将运行时 handle 作为持久身份。EOF/卸载只结算本地资源及自己的 pending login，并尽力 detach；显式 close/delete 才触发原生业务关闭。

可靠业务输出按序列化大小预留至 writer flush，预算 16 MiB；SDK 自行生成的协议错误不计入此预算。单 frame 16 MiB，单 frame 五秒内未 flush 则结束连接。恢复失败先生成已知请求的终态并限时排空输出；公开恢复截止用例验证响应在退出前送达。CLI 在插件自有工作已回收后，不等待被对端堵住的 OS stdio blocking worker，避免退出挂住。该宿主退出处理不改变共享 daemon 的执行生命周期。
