# Dim Rust CLI 项目剥离与发布方案

状态：待评审。本文定义实施范围与交付合同；npm发布工作流与产物校验脚本已准备，工作流保持禁用。源码提取、插件桥接和实际发布流程尚未完成。

日期：2026-10-13。

## 1. 交付目标与仓库

从 Agent v4 分支提取能独立构建、运行和发布的 Rust CLI 项目。CLI 和 Agent 核心用 Rust 实现，首版提供简单 scrollback；Dim OAuth 作为闭源 TypeScript rutis Plugin 独立维护，通过受信任的发布 CI 注入正式安装包。

| 项目 | 固定约定 |
| --- | --- |
| 源仓库 | `nextim/dim-agent`，分支 `feat/agent-v4` |
| 源码 checkout | `/Users/zerob13/Documents/workspace/temp/dim-agent-temp` |
| 本文核对提交 | `5df1149f430a427c334fc818634566bc05c5e16c` |
| 公共代码开发与评审仓库 | [nextim/dimrust-cli](https://gitlab.bfbops.com/nextim/dimrust-cli) |
| 公共代码本地 checkout | `/Users/zerob13/Documents/workspace/temp/dimrust-cli` |
| 开源目标 | [arcships/dim](https://github.com/arcships/dim) |
| 闭源插件仓库 | [nextim/dim-oauth-plugin](https://gitlab.bfbops.com/nextim/dim-oauth-plugin) |
| 闭源插件本地 checkout | `/Users/zerob13/Documents/workspace/temp/dim-oauth-plugin` |
| 产品与 npm 包 | `dimcode` |
| 安装后的命令 | `dim`、`dimcode`，指向同一个 Rust CLI |
| Rust crate | 保留现有 `dim-*` 名称；新增薄入口 `dim-cli` |
| 公共源码许可证 | MIT，实施时统一根文件、Cargo 与 npm 元数据 |
| CI | self-hosted负责构建与内网插件注入；GitHub公共runner负责npm OIDC发布，发布工作流预先准备但不启用 |

实施前重新检查源分支工作区、HEAD 和任务完成状态，将选定的完整提交 SHA 固定为提取基线。提取和验证使用该 SHA，不在实施中追踪移动的分支 HEAD。若包含本次核对提交以外的工作，先确认其交付状态并更新基线记录。

GitLab 保存开发、评审及内部发布运维资料；GitHub 接收公共源码和适合公开的使用、开发与扩展文档。本文包含内部仓库和本地路径，作为 GitLab 的内部实施资料，不直接纳入 GitHub 公开导出。闭源源码和内网下载配置不进入公开 Git 历史。

## 2. 范围与排除项

| 内容 | 处理方式 |
| --- | --- |
| `rust/crates/dim-agent-types` | 保留公共事件、数据及协议约定 |
| `rust/crates/dim-agent-runtime` | 保留 Timeline、Dispatcher、Agent/Session、装配与生命周期 |
| `rust/crates/dim-agent-capabilities` | 保留已实现的输入、模型、工具、审批、上下文、Skills、协作等能力 |
| `rust/crates/dim-agent-aimux` | 保留模型接入与请求转换 |
| `rust/crates/dim-app-server` | 保留通用 Provider、RPC、本地传输、存储、MCP；移出 Dim 专属业务 |
| `rust/vendor/aimux` | 保留当前使用的固定源码、补丁及第三方声明，不同时升级上游 |
| Rust 流程测试、fixtures 与 `xtask` | 迁入与留下的接口、存储及用户流程相关的部分 |
| 新 Rust CLI | 提供参数、终端 I/O、会话客户端和本地服务端启动/连接 |
| OpenTUI、Solid、TS CLI、两类旧终端界面 | 不迁入；不保留旧入口回退 |
| 旧 TS Agent SDK、agent-v3、TS agent-v4 执行核心 | 不迁入；需要的静态数据和协议样本迁到实际拥有方 |
| Desktop、Web、移动端、旧 Remote 产品 | 不迁入 |
| Dim OAuth 实现、Dim 目录规则及服务配置 | 归独立闭源插件仓库 |
| npm wrapper | 只保留启动平台 Rust 产物所需的 JavaScript |
| 根 monorepo 构建工具与多品牌发布链 | 按新项目需要收缩，不复制整套 pnpm/turbo/changesets 工作区 |

首版 CLI 暴露本方案列明的交互和命令。Rust 核心中已有但 CLI 尚未展示的能力仍可保留在服务端；不能因保留了内部能力就宣称终端已经提供对应面板。

本次不实现全屏界面、完整 Markdown 渲染、附件交互、多行编辑器、执行中排队编辑、插件市场、自动更新、Rust 动态库 SDK 或远程节点。账号插件的跨语言调用不要求迁移全部业务事件或完成原项目的多语言能力体系。

## 3. 代码布局与现有耦合

将现有 Rust workspace 移到新仓库根目录，更新所有相对路径。公共项目不再维护前端 workspace。

```text
dimrust-cli/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── crates/
│   ├── dim-cli/
│   ├── dim-app-server/
│   ├── dim-agent-types/
│   ├── dim-agent-runtime/
│   ├── dim-agent-capabilities/
│   └── dim-agent-aimux/
├── vendor/aimux/
├── tests/
├── xtask/
├── npm/
├── scripts/
├── docs/
└── .github/workflows/

dim-oauth-plugin/
├── package.json
├── pnpm-lock.yaml
├── src/
├── tests/
├── scripts/
└── docs/
```

插件用 pnpm 管理构建依赖并提交锁文件，运行包不要求用户安装 pnpm。私有源码不位于公共仓库的子目录、submodule 或 npm workspace 中。

以下为核对提交中需要实际拆开的依赖，实施时追踪所有调用者、测试、fixtures、exports 与发布脚本：

| 源路径 | 已发现的问题 | 交付处理 |
| --- | --- | --- |
| `rust/crates/dim-app-server/src/lib.rs` | 产品装配直接注册 `DimOAuthPlugin` | 改为由插件配置贡献托管供应商，不强制提供 Dim |
| `rust/crates/dim-app-server/src/provider.rs` 及 `provider/` | 直接依赖 `DimOAuthService`、Dim 账号快照、目录刷新和端点规则 | 保留通用目录/选择/模型解析，Dim 业务由插件提供 |
| `rust/crates/dim-app-server/src/connection.rs`、`protocol.rs` | RPC 方法、通知和返回类型绑定 `crate::dim_oauth` | 公共操作按供应商 ID 路由到通用账号合同，不跨边界暴露私有 Rust 类型 |
| `rust/crates/dim-app-server/src/model_catalog.rs` | 编译时读取旧 Agent 包 JSON，包含 Dim 专用目录投影 | 通用资源归公共 crate；Dim 投影规则归插件 |
| `rust/crates/dim-app-server/src/dim_oauth.rs` | 当前 Dim 登录和 token 生命周期为 Rust 实现 | 用作行为合同及流程验收参考，不迁入公共项目 |
| `packages/agent/src/provider/catalog/` | 部分 JSON 被 Rust `include_str!` 引用 | 将必要通用数据放到 Rust 自有资源目录；不保留旧包占位 |
| `rust/crates/dim-agent-types/scripts/compat.mjs` | 兼容样本生成依赖 TS 核心和共享包 | 保留必要的固定 wire fixtures；日常构建/测试不依赖旧项目生成器 |
| `rust/xtask` 和 Rust 测试 | 生成目录、架构规则和 `include_str!` 可能依赖原目录结构 | 更新输出路径和边界规则；不手工改生成图冒充当前装配 |
| 原 CLI 构建与 `npmbin` | 多品牌、Bun、媒体、sandbox、私有静态资源链路混合 | 仅复用留下的产物校验、包装和发布流程，不迁入旧构建依赖 |

当前基线使用 `rutis` / `rutis-loader` 0.10.0。Agent、Session、分支的 loader 装配已经存在；Node runtime、账号跨语言桥接以及进程插件全面迁为 loader 行尚未完成。实施不能将这些目标合同写成现有能力。

## 4. 模块关系、状态归属与生命周期

```text
Rust CLI (dim / dimcode)
        | private local RPC
        v
Rust App Server
        ├── Agent v4 runtime and capabilities
        ├── SQLite / model catalog / provider selection
        ├── Aimux model request execution
        └── rutis loader and service bridge
                    |
              Managed Node runtime
                    |
              Dim OAuth Plugin
```

该图表达本方案的目标边界。实现装配图由 `xtask` 从实际代码生成。

| 拥有方 | 状态与资源 | 不承担的工作 |
| --- | --- | --- |
| `dim-cli` | 终端输入输出、当前会话 ID、请求 ID、事件游标、本地连接及登录展示 | 不执行 Agent loop，不自行裁决权限，不刷新或存储 token |
| App Server | 本地连接、通用 Provider 注册与选择、SQLite、插件配置与业务准入 | 不包含 Dim 域发现、专属目录解释或 token 状态机 |
| Runtime / capabilities | 执行、Timeline、上下文、工具、审批、取消与持久化终态 | 不维护终端视图或另建插件通信通道 |
| Aimux | 当前模型请求和流，沿用现有 ModelHost 合同 | 不保存账号，不建立第二个执行循环 |
| rutis Node runtime | JS 进程、跨语言服务和插件生命周期 | 不创建独立于 rutis 的插件管理框架 |
| Dim OAuth Plugin | 登录尝试、PKCE、token 文件、刷新与退出、Dim 目录和请求凭据租约 | 不读取终端输入，不直接向 scrollback 写内容，不拥有 Agent Session |

账号插件是进程级服务。关闭某个 Session 不取消其他客户端的登录；插件卸载/进程关闭才清理它拥有的轮询、订阅和文件资源。同一种语言使用已有 rutis 运行时机制，不为每个登录尝试或 Session 创建 Node 进程。

Node runtime 崩溃时撤下其提供的账号服务，使用 Dim 的请求明确结束或进入既有恢复流程；不自动重放 token exchange、refresh 等可能已产生副作用的调用。其他供应商仍可使用，不能把账号插件故障解释为整个 daemon 或全部 Session 必须退出。

退出和重载先结算受理的业务，再释放所属资源；跨语言等待者取消不能丢弃已经成功轮换的 token。关闭/重载的代次变化使旧句柄失效，迟到响应不能覆盖新账号或新插件状态。

## 5. TS OAuth 插件接口与私有包

### 5.1 接口语义

公共接口只覆盖实际接线需要的账号、模型目录和凭据使用；放在现有公共类型或宿主模块中，不新增独立的通用 SDK 项目。下表定义待实现的操作语义，最终方法名和 wire 字段需随接口实现一次定稿。

| 操作 | 输入与结果 | 边界 |
| --- | --- | --- |
| 供应商发现 | 插件贡献 ID、显示名称、登录方式及支持操作 | 没有插件时不出现 Dim 供应商；公共配置不保存其服务地址 |
| 账号状态 | 供应商 ID → 非秘密账号快照与可用状态 | 状态不含 access/refresh token；损坏存储明确失败 |
| 开始登录 | 登录方式、必要输入及已绑定的回调信息 → attempt ID、授权 URL 或设备码、截止时间 | 回调监听和终端显示归 CLI；PKCE、域发现归插件 |
| 完成/查询/取消登录 | attempt ID 与回调信息 → 状态或非秘密快照 | 保留 state 校验、超时、重连查询与取消结算 |
| 退出/显式身份恢复 | 当前供应商与账号操作 → 非秘密快照 | 退出不删除其他供应商数据；恢复不隐式改变原身份授权 |
| 模型目录 | 当前账号身份/代次 → 已归一的公共模型描述、可用性与请求路由信息 | Dim 原始配置的解释由插件完成，通用消费方不读取私有字段 |
| 请求凭据租约 | 供应商、当前账号与模型请求 → 有代次的短期租约及请求鉴权材料 | 只进入内部请求路径，refresh token 不出插件，不返回终端 RPC |
| 凭据拒绝与释放 | 请求租约、模型错误阶段 → 恢复结果或最终拒绝 | 保留一次受控恢复及最终拒绝的现有业务含义，不盲目重试 |

Rust 侧使用 rutis 的 `HostDispatch` / 命名服务能力对接 TS 插件。RPC 接入层仅承载这些供应商操作，不能通过类型断言、默认 Dim ID 或空账号状态掩盖插件缺失。秘密服务不进入客户端可调用的方法目录。

账号命令不在公共 wire 中依赖 `DimOAuthService`、Dim 专用错误枚举或私有 TS 类。供应商 ID 是输入数据，显示内容来自已验证的描述。已有非 Dim 供应商保持自身合同；本次不重写其认证业务。

CLI/RPC 输入、插件响应、网络响应和持久化读取各自在真实的外部边界校验一次，之后用明确类型传递。缺少服务、错误格式、未知账号 schema、插件兼容性错误分别报告，不解释成未登录。

### 5.2 存储与迁移

插件是 Dim token 与账号元数据的唯一写入方。复用既有 TS 账号存储和 OAuth 业务中适用的实现，迁入独立仓库后去除旧 Agent SDK、DSH/Cordis 宿主及 UI 依赖，不同时保留另一份 Rust Dim writer。

需要保持以下合同：

- 文件读取边界严格解析；锁内读改写，私有权限和原子替换；保留共享账号文件中其他供应商的字段。
- 登录开始捕获账号代次；token exchange 后确认身份并通过 revision/CAS 提交。迟到登录、refresh、logout 不能覆盖新状态。
- 凭据拒绝按本次租约比较处理；网络、429、5xx 保留账号，不清除尚未被拒的新 token。
- refresh 一旦取得合法轮换结果，负责结算和持久化的插件任务不随等待者取消而丢弃结果。
- 正常读取、显式身份恢复与受控重置有不同业务含义；保留现有格式门禁、备份和拒绝覆盖规则，不做通用失败兜底。
- token、授权码和 PKCE verifier 不进入 Timeline、日志、终端记录、SQLite 模型配置或发布包。

产品按 `dimcode` 命名和数据目录约定实施，但首次验收使用隔离的 `--home`。已有 `.dimcode`、`.dim-alpha` 数据不自动移动、覆盖或改写；沿用旧登录和会话只在明确的格式兼容条件下显式接入。持久化格式兼容检查保留在公共流程测试中。

### 5.3 私有插件产物与本地开发

插件仓库独立锁定依赖，构建得到可直接加载的 ESM 运行包。运行包至少包括编译后的入口、实际运行依赖及插件清单；不携带 TS 源码、source maps、测试、开发配置或用户账号文件。清单记录插件版本、公共宿主接口版本、rutis SDK 版本、入口及文件摘要。

私有插件实现不提交到 `dimrust-cli`，不成为公共 Cargo/npm 安装过程的依赖。正式分发允许将闭源插件的运行产物一起打入安装包，本方案不展开插件许可证选型。

本地用同一装载合同加载私有仓库的构建目录，通过显式开发路径配置定位插件和 Node runtime。私有目录存在不是公共源码构建的前提。开发脚本先构建插件再启动公共 Rust CLI；不下载正式内网包，也不构建另一份私有 App Server。

## 6. Rust scrollback 首版合同

`dim-cli` 是现有 App Server 的 Rust 客户端，复用 Unix socket / Windows named pipe 与公开 RPC。默认命令启动或连接产品服务端后进入交互；服务端发现使用通用产品能力和协议版本，不能用 `account.dim.status` 的存在判断产品身份。

```text
dimcode · model: selected-model
workspace: /path/to/project
session: abc123

> Explain this project

Assistant:
This project contains ...

[tool] read README.md
[done] read README.md

[approval] exec: cargo check
Allow once? [y/N] y

Assistant:
The main entry point is ...

> _
```

| 流程 | 首版行为 |
| --- | --- |
| 输入 | 单行提交一轮，结束后显示下一提示符；运行中不建立客户端输入队列 |
| 输出 | 模型文本增量写入终端；工具状态输出单独的记录；不重绘已有 scrollback |
| 工具审批 | 展示名称、必要参数摘要与风险，允许一次或拒绝，空输入默认拒绝；决定仍由服务端受理 |
| 中断 | 运行中 `Ctrl+C` 请求停止并等待资源结算，再显示提示符 |
| 退出 | 空闲时 `Ctrl+C` 或 EOF 结束 CLI；关闭连接不冒充删除/关闭会话 |
| 恢复 | 通过明确会话 ID 恢复，展示必要历史并保留真实 sequence 游标 |
| 配置 | 配置文件、环境变量及少量命令管理模型；没有模型时明确给出配置路径 |
| 登录 | 供应商命令触发，CLI 显示授权 URL/设备码、处理回调和结果，不读取 token |
| 非交互 | 单次提示词或 stdin 输入，stdout 输出文本，stderr 输出诊断，返回明确退出码；需要审批时明确失败，不永久等待 |
| 失败 | 缺少配置、插件不可用、连接断开、审批取消和模型错误都有可理解的结果；不静默切回旧核心 |

终端只有一个输入拥有方和一条串行输出路径；Node 插件不继承 stdin，也不直接写业务内容到 stdout。模型、工具、审批结果由 Rust CLI 排序展示；诊断进入 stderr。终端控制字符在展示边界处理，避免模型或工具输出注入终端控制操作。

首版不要求高级行编辑、多行输入、输入历史、面板或主题系统。使用现有依赖和标准终端行为；只有流程需要的缺口才添加依赖。命令参数的实际拼写和退出码表在 CLI 实施时写入公开使用文档，不继承旧 CLI 中未接 Rust 的命令承诺。

本地端点仍遵守私有权限、服务端身份与连接校验。CLI退出默认 detach；是否关闭服务端只由明确的服务端管理操作决定。未知协议或不兼容 daemon 明确报错，不能自动删除端点或杀死正在使用数据目录的进程。

## 7. 发布包、内网注入与版本

### 7.1 平台和运行包

目标平台为 macOS arm64/x64、Linux arm64/x64、Windows x64。支持声明以实际原生构建和安装冒烟验收为准。Linux libc、macOS 最低版本与 Windows 系统/运行库要求由构建基线确认，不因 Rust 编译成功就宣称已支持。

含 OAuth 的正式包随附固定版本的 Node runtime、rutis JS runtime 和私有插件。JS运行时按随包绝对路径启动，不从用户 PATH 猜测版本；插件不依赖用户机器联网安装 npm 依赖。公共源码未配置插件时不启动 Node，API Key 路径仍可用。

```text
dimcode-<platform>-<arch>/
├── bin/dimcode[.exe]
├── bin/dim-app-server[.exe]
├── runtime/node/
├── runtime/rutis/
├── plugins/dim-oauth/
└── release-manifest.json
```

该目录表达打包职责。Linux/macOS 可用 `.tar.gz`，Windows 使用 `.zip`；npm 平台包从同一份 staging 目录生成。最终目录在首次打包验证时固定一次。

Node、rutis runtime 和保留的其他运行资源从固定、可校验的来源取得；不复用旧 monorepo 的私有静态依赖下载器。产物不含 OpenTUI/Bun 终端资源、旧 Agent 包、未使用媒体链路或 Desktop 进程。

### 7.2 CI 输入

内网地址由 GitHub Environment variables 注入，工作流不硬编码它。建议变量：

| 输入 | 合同 |
| --- | --- |
| `DIM_OAUTH_PACKAGE_URL` | 固定版本运行包的内网地址；构建时下载，不作为用户运行时地址 |
| `DIM_OAUTH_PACKAGE_SHA256` | 已确认的包摘要，校验失败终止发布 |
| 下载凭据（如需要） | GitHub Environment Secret，不放变量、命令回显、下载 URL 查询参数或产物清单 |
| npm 发布身份 | GitHub公共runner通过OIDC取得临时身份；不向构建或发布作业注入长期npm token |

普通源码/PR验证不依赖上述输入。包含 OAuth 的正式发布作业明确要求 URL 和摘要；缺失、下载失败、摘要/清单/兼容版本错误均终止发布，不悄悄发布缺少 OAuth 的同版本产物。

下载和解包只写 staging 目录，检查归档路径不能越界，不能覆盖仓库或包含来自构建机器的任意链接。先校验再加载代码。stage 中只有运行所需文件；构建结束清理私有下载残留，不把它们缓存到公共作业或上传成源码附件。

同一版本的所有平台使用同一私有插件版本与摘要。重试使用同一输入和原产物；URL 对应内容变化必须触发摘要失败，不能在同一版本中混用插件。

### 7.3 版本、来源与npm发布

一个产品 SemVer 驱动 Rust workspace、CLI版本输出、npm入口包和平台包。GitHub tag、产品版本及清单必须一致。插件独立版本，由接口兼容合同决定是否需要同步升级；同一分发记录固定其版本及摘要。

复用 `dimcode` 入口包和 `dimcode-<platform>-<arch>` 平台包模式，入口只负责定位二进制、转发参数/信号及退出码。入口的 optionalDependencies 精确指向同版本平台包；先完成全部平台包发布，再发布入口包。预发布使用 `next`，稳定版使用 `latest`。

发布清单至少记录公共源码提交、产品版本、目标平台、工具链/Node/rutis版本、插件版本与摘要、每个产物摘要。不写内网地址、下载凭据或账号数据。包含闭源运行产物的正式包能够按固定输入重建，不承诺仅用公共源码就能重建闭源插件内容。

公开源码导出只包含白名单中的源码、资源和公开文档，不导出旧 Git 历史、内部实施资料、staging或私有插件源码。提取开始后由新仓库拥有公共代码，插件仓库拥有 Dim 业务；不建立长期双向源码复制。

## 8. CI、认证与发布流程

公共仓库使用GitHub格式的工作流。`ci.yml`和`release.yml`的构建、内网插件下载及平台验收运行在self-hosted；npm发布在GitHub公共runner `ubuntu-latest`上执行，读取已生成的包，不重新构建，不访问内网。GitLab用于分支和MR评审。公开源码同步方式在开源交付阶段固定，源tag对应的提交需能追溯到GitHub实际checkout。

| 工作流 | 输入与职责 | 完成条件 |
| --- | --- | --- |
| `ci.yml` | 公共源码、锁文件；fmt、Clippy、相关流程测试、crate边界和构建 | 不访问私有插件；单靠公共内容通过 |
| `release.yml` | self-hosted：固定tag、平台矩阵、Environment输入；构建、staging、安装验收，上传`npm-packages` artifact及Release产物 | 所有平台产物满足同一清单；上传包不带内网地址和凭据 |
| `npm-publish.yml.disabled` | GitHub公共runner：读取同仓库已成功构建的artifact，验证来源/摘要/版本，使用OIDC先发布平台包再发布入口包 | 默认不被GitHub识别为工作流；后续明确启用并配置trusted publisher后才能发布 |
| 私有插件构建 | 插件仓库、锁文件和接口版本；TS构建、账号流程检查、运行包生成 | 产生可直接加载的固定版本包，供内网制品服务接收 |

```text
Self-hosted: fixed source and version checks
          |
Self-hosted: native Rust builds
          |
Self-hosted: download / verify private plugin and runtime
          |
Self-hosted: stage platform payloads
          |
Self-hosted: archive checks and installation smoke tests
          |
Upload verified npm-packages artifact
          |
GitHub-hosted: validate artifact and publish platform packages
          |
GitHub-hosted: publish dimcode wrapper using OIDC
          |
Complete GitHub Release after package publication
```

构建、打包、校验脚本可本地调用，CI只编排。发布权限只授予GitHub公共runner上的受信任发布作业；self-hosted构建环境只有内网资源读取和产物上传所需权限。公共PR不得在可访问内网和私密环境的runner上执行任意构建代码，普通验证使用隔离环境。

GitHub Release使用带版本的压缩包、SHA-256和清单，与npm复用同一批二进制，不另做一次构建。发布失败保留可核对的进度和产物；不覆写已发布的同版本内容。仅更新dist-tag时仍核对对应版本产物。

### npm发布工作流的禁用与启用合同

发布工作流保存为`.github/workflows/npm-publish.yml.disabled`。GitHub只识别`.yml`/`.yaml`工作流，因此此文件不注册、不会自动触发，也不能被手动dispatch。当前阶段不启用或执行任何发布job。

后续明确启用时将文件改名为`npm-publish.yml`并进入公开仓库的默认分支，才成为GitHub工作流。准备文件只声明`workflow_dispatch`，不声明push、tag push、release、workflow_run或schedule触发；启用后也不因代码、tag或构建完成自动发布。dispatch时指定release构建run ID与release tag，并从该tag执行。

发布前置条件为：公开仓库`arcships/dim`、同tag成功完成的`release.yml`构建、`npm-release` Environment，以及入口包和五个平台包各自的npm trusted publisher。每个publisher绑定仓库、最终工作流文件名和Environment，并允许`npm publish`；首次包创建和维护者权限在正式启用时确认。

工作流使用Node 24，并确保npm满足trusted publishing最低版本要求；job获得`id-token: write`，不需要`NPM_TOKEN`或`NODE_AUTH_TOKEN`。self-hosted不承担npm发布，因此token直接发布停用或stage-only晋升不作为本项目的发布方案前提。

### 构建到发布的artifact合同

self-hosted在所有产物检查与安装验收通过后，将`npm-packages`上传到同一GitHub仓库的构建run。artifact含六个`.tgz`及`release-manifest.json`，不包含下载鉴权、内网地址、源码工作区或额外构建缓存。

| 字段或内容 | 合同 |
| --- | --- |
| `release-manifest.json.version` | 与dispatch tag去掉`v`后的SemVer一致 |
| `release-manifest.json.sourceCommit` | 与dispatch tag及构建run的完整提交SHA一致 |
| `release-manifest.json.npmPackages` | 恰好五个平台包和一个`dimcode`入口包，每项包含`name`、`file`、`sha256` |
| 归档文件名 | `<package-name>-<version>.tgz`，仅允许本目录内的普通文件 |
| 归档中的`package/package.json` | 名称/版本与清单一致；平台包os/cpu正确，入口精确引用相同版本平台包，`dim`/`dimcode`使用同一wrapper |
| `repository.url` | 所有包均为`git+https://github.com/arcships/dim.git`，符合OIDC仓库绑定 |

公共runner先核对构建run来自同一仓库、预期`release.yml`、受信任的push或手动事件，已完成且成功，提交与dispatch tag一致，再下载指定artifact；下载action的摘要错误直接失败。随后`scripts/publish-npm.mjs`在任何registry写入前检查全部包的内容身份、版本和SHA-256，先发全部平台包，最后发入口包，禁用包生命周期脚本。稳定版用`latest`，预发布用`next`。

本地检查使用`node scripts/publish-npm.mjs DIRECTORY VERSION SOURCE_COMMIT`，默认只校验；只有显式`--publish`才调用npm。发布失败直接中止，不覆盖现有版本；重新发布前人工核对已成功包与原产物后继续处理。

当前已有的发布脚本验证使用真实tarball，只替换registry写入边界。验证入口为`node --test scripts/publish-npm.test.mjs`，不触发GitHub工作流或访问npm registry。正式启用前仍需实际构建artifact和OIDC发布演练。

依据：[npm trusted publishing](https://docs.npmjs.com/trusted-publishers/)、[GitHub workflow syntax](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax)、[rutis Rust host embedding](https://github.com/arcships/rutis/blob/main/docs/guide/rust-host.en.md)。rutis文档中的示例版本可能落后于源码，实施以固定的0.10.0 API和实际流程验证为准。

## 9. 交付任务与依赖

任务边界按可独立验收的模块和集成流程划分；不把每个文件或每条问题拆成单独任务。

| ID | 交付 | 依赖 | 核心验收 |
| --- | --- | --- | --- |
| D01 | 固定源码基线和迁入白名单，提取独立Rust workspace，整理资源/fixtures/工具链 | 本方案确认 | 原目录不可用时仍能构建、运行公共测试；无旧核心路径依赖 |
| D02 | 通用Provider/账号服务合同，移除Dim强制依赖，公开接口和秘密端口分离 | D01 | 无插件的API Key流程可用；插件缺失/错误不会变成伪造账号 |
| D03 | 私有TS OAuth插件及独立运行包 | D02接口确定 | 登录、刷新、退出、恢复和存储竞争满足行为合同；独立打包 |
| D04 | rutis Node runtime与Provider真实桥接，本地插件装载 | D02、D03 | 从插件取得目录/短期凭据并由Aimux执行；取消、401和runtime故障正确结算 |
| D05 | Rust CLI与本地服务端连接、scrollback和账号命令 | D01；账号接线依赖D04 | 输入、输出、工具、审批、中断、恢复、非交互及登录跑通 |
| D06 | self-hosted原生构建/内网注入/npm包装，GitHub公共runner OIDC发布与Release接线；发布job保持禁用直到明确启用 | D04、D05 | 五平台产物检查与实际安装；artifact交接和失败门禁；同版本一致 |
| D07 | 使用/开发/扩展/发布文档、MIT及第三方声明、GitHub公开导出 | D01—D06 | 公开源码和正式分发物分别符合范围；版本和来源可追溯 |

D02先定义接口，随后D03与D05的无账号终端部分可以独立推进。重叠的Provider/RPC装配修改串行处理；真正并行实施时使用项目既有docs-sprint组织方式，核对依赖后再决定是否拆给agent。当前交付包含可供MR评审的方案，以及默认禁用的npm发布工作流和独立产物校验脚本。

## 10. 验收与消融检查

验证优先复用现有公开流程和持久化测试。只有新代码、失败或未解决疑点才追加/扩大检查，不反复跑全部monorepo测试。私有辅助函数不新建独立测试；跨语言、持久化、安全和发布产物边界保留必要合同检查。

| 边界 | 必须留下的证据 |
| --- | --- |
| 纯公共构建 | 干净checkout，无源monorepo、私有目录和内网变量，锁定依赖构建并用API Key执行真实模型/工具 |
| Rust用户流程 | 审批默认拒绝、中断结算、退出detach、会话恢复不重复输出、无模型与daemon不兼容错误 |
| 登录与账号 | 浏览器/设备码、回调state错误、超时/取消、refresh竞争、退出与迟到登录、未知schema、其他provider字段保持 |
| 跨语言集成 | 实际rutis runtime，不用mock代替：载荷校验、服务缺失、runtime退出、代次变化、取消及未知副作用结果 |
| 运行包 | 在未预装Node/Rust的验收环境运行；无运行时npm install；插件日志不破坏终端或泄漏凭据 |
| CI下载 | 缺变量、网络失败、错误摘要、归档越界及接口版本不兼容使发布终止 |
| 分发 | 真正安装npm tarball与Release包，检查命令、二进制平台、版本、退出码、插件和runtime文件 |
| 公开边界 | 检查Git跟踪内容、公开导出、npm tarball、Release压缩包；私有TS/source maps、账号文件、内网凭据和旧核心缺席 |
| 版本与来源 | tag、Cargo、版本输出、npm平台包、入口包、插件摘要与清单一致；重试不混用输入 |
| 发布禁用与交接 | `.yml.disabled`不注册或触发；公共runner拒绝不同提交、未成功或非release工作流的artifact；任一包不合法时零registry写入 |

消融检查逐项审查是否还存在旧CLI包装、无用多品牌分支、占位共享包、重复账号writer、第二套状态机/事件总线/插件管理器、不必要的同步缓存与兜底。删去无助于首版流程的结构；runtime桥接、凭据代次、文件锁和产物校验属于真实边界，保留并说明其用途。

交付完成的判定是：公共源码可独立构建和使用；私有插件可独立构建并在本地接入；self-hosted能够从固定内网包生成可安装的正式产物；npm发布job已准备且保持禁用。实际发布仅在明确启用后，由GitHub公共runner通过OIDC执行；源码范围、账号语义及启用前提都有验证证据。

## 11. 实施前需要验证的具体事项

产品边界、命名、仓库、MIT与TS私有插件形式按本方案执行。以下是有明确验证对象的实施工作，不要求重新讨论已经确认的产品范围：

1. 固定源码提交后，核对所有跨旧目录依赖及原项目未交付任务，避免把目标合同当已实现源码。
2. 用固定rutis版本验证Node服务注册、异步请求取消、实例代次和runtime清理；再固定账号接口的字段和错误语义。
3. 验证选定Node/rutis运行包在五个平台的可启动性，并记录其固定版本及摘要。
4. 确认self-hosted runner标签、隔离方式、平台工具链及内网制品Environment输入；验证artifact向公共runner交接，并在明确启用时配置各npm包的trusted publisher。
5. 确定第一版产品版本、公开tag及现有`dimcode`平台包的版本升级方式；不覆盖旧版本，不默认改写已有用户数据。
6. 在公开导出中保留必要第三方来源和声明，排除本文等内部运维资料；公共仓库MIT文件的版权主体按项目归属填写。
