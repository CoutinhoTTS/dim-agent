# 升级到 rutis 0.19.0

[English](../../design/rutis-0.19-upgrade.md)

状态：草案。

规则：[设计规则](../../architecture/rust-agent/principles.md)、[ADR 002](../adr/002-plugin-scope.md)、
[ADR 006](../adr/006-plugin-contract.md)、[Dim 如何使用 rutis](rutis-usage.md)。

本次升级把 `rutis`、`rutis-loader`、`rutis-bridge`、随附的 Node 运行时、仓库内的 TypeScript 插件以及
Python SDK 从 0.17.1 升到 0.19.0。rutis 0.18 和 0.19 都有不兼容变更（[0.18 迁移指南](https://github.com/arcships/rutis/blob/v0.19.0/docs/migration-0.18.md)、
[0.19 迁移指南](https://github.com/arcships/rutis/blob/v0.19.0/docs/migration-0.19.md)）。本次升级不保留兼容路径。
rutis 删掉的 API，Dim 一律改用 rutis 规定的新写法，不加重建旧 API 的包装层。

## 1. 范围，以及为什么一步做完

| 项 | 现状 | 目标 |
| --- | --- | --- |
| `Cargo.toml` 工作区依赖 | `rutis`、`rutis-bridge` 0.17.1；`rutis-loader` 0.17.1，带 `node`、`python` | 0.19.0；`rutis-loader` 只带 `node`（0.19 没有 `python` feature，`node` 会启用 `runtimes`） |
| 新增依赖 | — | `rutis-host` 0.19.0，加在 `dim-app-server`，以及启动运行时的测试和示例中 |
| `xtask` | `rutis-bridge`，带 `exports` | `rutis-types` 0.19.0 |
| `runtime/package.json` 及锁文件 | `@arcships/rutis-runtime`、`@arcships/rutis` 0.17.1 | 0.19.0；另加开发依赖 `@arcships/rutis-host` 0.19.0（M10） |
| `runtime/plugins/{goal,scheduler,session-title,workflow}/package.json` 及锁文件 | `@arcships/rutis` 0.17.1 | 0.19.0 |
| `npm/package.json` 的 peer 依赖 | `@arcships/rutis` `~0.17.0` | `~0.19.0` |
| `runtime/python-requirements.txt` | `rutis==0.17.1` 及其哈希 | `rutis==0.19.0` 及其哈希 |
| 作者指南的固定版本 | [作者指南](../../typescript-plugin-authoring.md) | 0.19.0 |

0.19 使用会话协议 4，握手时会拒绝 0.18 及更早的对端，两个方向都是如此。因此 App Server、Node 运行时、
Python 运行时和交互式 `dimcode` 进程（UI leaf，M8）必须一起升级。只升级 Rust 侧会让所有 TypeScript 和
Python 行都无法连接。

不在范围内：Go、Swift、Kotlin 运行时，dylib 插件，远程运行时与节点连接行（`rutis-host/dial`、
`rutis-host/listen`），`rutis-host` 命令，以及 Cordis 宿主。Dim 都不使用。`rutis-migrate` 停在 0.18，不使用它。

## 2. 0.17.1 到 0.19.0 之间的变化

| 版本 | 与 Dim 相关的内容 |
| --- | --- |
| 0.17.2 | 无 API 变化。坏值只让这一次调用失败；运行时退出不再连累其他行；10 秒后报告 `StillWaiting` |
| 0.18 | 错误文案带错误码（`session::Error::Binding`）；默认错误 sink 写标准错误；配置按 schema 校验并填入默认值；`reconcile` 不再等待仍在启动的运行时；带方法的对象可以传给每种语言 |
| 0.19 | 本地运行时是一个会话的对端；链接、peer、传输和身份这一层整个删除；服务目录共享所有未登记的名字；运行时的行遵守 `isolate`；运行时的每次 emit 都经过宿主；`LoaderPlugin` 提供服务 `loader`；SDK 测试工具运行在真实的 `rutis-host` 上；要求 Python 3.12 及以上；线上错误名 `CordisError` 改为 `Error` |

### 2.1 Dim 调用的内核 API

| 0.17.1 | 0.19.0 |
| --- | --- |
| `Ctx::waterfall_sync(event, terminal)` | 删除：改用 `waterfall`（异步）或同步的 `bail` |
| 同步监听器里的 `SyncCall::next()` | 总是返回 `Err(WaterfallNext)`：同步监听器后面没有链可以继续执行 |
| `Ctx::root()` 的默认错误 sink | 把 `StartFailed`、`RuntimeDown`、`StillWaiting` 等各类错误写到标准错误 |
| `ErrorKind` | 新增 `StartFailed`、`RuntimeDown`、`NotServedByName`、`StillWaiting`；`DisposeWaitError::TimedOut` 新增字段 `stuck` |

`Ctx`、`DynPlugin`、效果、`Key`、`TypeKey`、`Ctx::isolate`、监听器、`on_sync`、`prepend` 和
`#[rutis::service]` 保持 0.17.1 时的契约。

### 2.2 Dim 调用的 bridge 与 loader API

| 0.17.1 | 0.19.0 |
| --- | --- |
| `rutis_bridge::runtime::LocalRuntime`（`node`、`python`、`named`、`interpreter`、`python_path`、stdio、`handle`） | `rutis_host::connect::SpawnPlugin::new(name, launcher, anchor)`，配合 `rutis_host::languages::{node, python}::launcher`；构造方法相同 |
| `RuntimePlugin`、服务 `runtime::Runtime` | 删除；状态通过 `RuntimeHandle::state()` 读取，取值不变 |
| `Stdio::Lines` | `rutis_host::output::lines(stream, prefix)` |
| `rutis_loader::runtime::RuntimeResolver::node` / `::modules`、`with_catalog`、`invalidate` | 改为一个 `rutis_loader::runtime::SessionResolver`（`npm(&catalog, row)`、`runtime(handle)`、`attach(&loader)`、`follow_row(row)`）；没有需要失效的缓存；npm 名字由 Node 解析 |
| `RuntimeRows`、`RowsSource`、`RuntimeRowsPlugin` | `SessionRows::key(row)`；`SessionResolver::follow_row` |
| `ServiceCatalog::share_registered_only()` | 删除：所有没有另行登记的名字都按名字共享 |
| `Loader::waiting_for(entry)` | `Loader::waits(entry)` |
| `Loader::set_overlay`、`overlays()` | 删除：在交给 `reconcile` 的层列表里放一层自己的 |
| `PeerPlugin`、`LinkConfig`、`LinkState`、`Features`、`Peer`、`peer_key`、`plugins.load` / `plugins.unload` | `Session::handshake(&ctx, channel)`、`Greeted`（`id()`、`handle()`）、`SessionHandle::run(plugin, config)`、`RowHandle::loaded()` |
| `Identity`、`StaticIdentity`、`IdentityPlugin`、`identity_key` | `rutis_bridge::LocalId(PeerId)`，每个内核一个 |
| `Transport`、`Dial`、`transport_key` | 删除：连接就是交给会话的一个 `Channel` |
| `transport::local::framed` | `pipe::framed` |
| `Closer::close(&str)` | `Closer::close(Close)` |
| `StaticCatalog::with(name, Described { .. }, factory)` | `StaticCatalog::with(name, Installed::new(factory).schema(..).version(..))`，提供在 `catalog_key()` |
| `rutis_bridge::build::Exports` | `rutis_types::Exports`，API 不变；生成文件的文件头变了 |
| `LoaderPlugin` 提供 `Loader` 和 mounter | 另外还提供标记服务 `loader`，以及 `catalog_key()` 下的会话插件目录 |

`host_key`、`HostDispatch`、`Build`、`ServiceCatalog`、`Builtins`、`Resolved`、`mounter_key`、
`session::Error` 和 `settle` 保持原有契约。

### 2.3 影响 Dim 正确性的行为变化

- **标准错误。** 默认错误 sink（0.18）和 loader 的配置警告都会写到标准错误。TUI 运行期间，终端归交互式
  `dimcode` 进程所有（M7）。
- **配置会被校验。** 0.18 起，每一行的配置在 `build` 之前按其 schema 校验并填入默认值。对象类型的 schema
  缺省配置会变成 `{}`，所以 Dim 的模块（`NoConfig`）能通过。配置不符合自身 schema 的 TypeScript 插件，
  现在会让它的行失败。
- **`reconcile` 不再等待仍在启动的运行时**（0.18）。这个运行时的行会等它就绪，不出现在报告里。
- **npm 名字由 Node 运行时解析**（0.19）。不认识的裸名字不再立刻失败，而是等待运行时的会话。
- **运行时的每次 emit 都经过宿主**（0.19）。Dim 的插件只监听 Rust 发出的事件。
- **运行时的行遵守 `isolate`**（0.19）。Dim 不在运行时的行上写 `isolate`；Session 和分支的 TypeScript
  部件是插件定义的副本，没有 `isolate`。
- **会话会导出和公布。** 一个会话会把非平台插件提供的所有进程级共享服务导出给对端，并把每个监听器的
  名字公布给对端。对端可以运行本端会话插件目录能解析的任何插件（0.19）。
- **错误文案变了，并带上错误码**（0.18）。Dim 没有按 rutis 的错误文案做匹配。Dim 手写的 dispatcher
  继续返回 `session::Error::Value`。
- **SDK 测试工具运行在 `rutis-host` 上**，被测插件必须是所在模块的默认导出（0.19）。
- **要求 Python 3.12 及以上**（0.19）。下载的运行时是 3.12.15，CI 用的也是 3.12。

## 3. Dim 的当前用法

在 70ad3835 上把所有 target 对 0.19.0 试编译一次，结果如下：

| Crate | 结果 |
| --- | --- |
| `dim-agent-runtime` | 2 个错误：`waterfall_sync`（`core/timeline.rs:318`）、`share_registered_only`（`names.rs:95`） |
| `dim-agent-types`、`dim-agent-capabilities`、`dim-agent-aimux`、`dim-agent-attachments`、`dim-agent-questions` 及其测试 | 修好上面两处后即可编译 |
| `dim-app-server` | 16 个错误：`lib.rs`（运行时的行、`NodeRuntime`）、`session_typescript.rs`、`runtime_manager.rs`、`plugin_manager.rs`、`ui_leaf.rs` |
| `tests/integration` | `extensions.rs`（一个 `LocalRuntime` 及其解析器）；其余测试都能编译 |
| 没编到、靠搜索定位的 | `dim-cli/src/leaf.rs`；`xtask/src/types.rs`；`dim-app-server/tests/transport_flow/ui_host.rs` 和 `session_typescript.rs:1674`；`transport_flow/{scheduler,workflow,process_typescript}.rs` 里的 `set_overlay`（9 处调用）；`examples/session_typescript_spike.rs` |

## 4. 决策

### M1. 宿主、运行时、leaf 和插件作为一次升级单元

§1 的所有固定版本放在一个提交里升级。握手会拒绝版本混搭，所以任何中间状态都构建不出能用的产品。
`runtime/plugin-sdk.test.mjs` 会打包 `npm/` 并对作者指南做类型检查，因此作者文档里的类型和 SDK 版本一起变化。

### M2. Timeline 的 `before` 链改用同步 `bail`

`Timeline::commit` 在追加守卫和提交锁之内运行，并在那里分配序号。所以这条链保持同步，不把提交路径改成
异步的 `waterfall`：

- 传出的 `BeforeCommit` 在一把锁里携带当前 draft。每个变形读取它；键匹配时调用自己的处理函数，把结果写回，
  记录自己这一步，然后返回"无结果"，这样 `bail` 会依次调用每个监听器。
- 变形用 `prepend` 注册，所以后注册的先执行。这与现在的顺序一致：先注册的中间件包住后注册的，后注册的先变形。
- 某个变形失败会让 `bail` 失败。这时提交使用原始 draft，不记录任何步骤，与现在相同。

私有名字 `dim/timeline/{instance}/before` 和 `Timeline::before` 保持原有契约
（[0.16 U2](rutis-0.16-upgrade.md#u2-分支机制通道使用私有名字)）。

### M3. 服务目录共享所有未登记的名字

删除 `share_registered_only`。已登记在服务目录里的名字保持原来的键。外来行注入、而服务目录没有登记的名字，
现在在 rutis 里按它的共享键等待，而不是交给运行时自己处理。[Dim 如何使用 rutis §3.4](rutis-usage.md#34-服务目录)
删掉关于 `share_registered_only` 的那句话。

### M4. 本地运行时是会话

- **启动。** Node 运行时 `session-node` 和 Python 运行时 `py` 都是 `SpawnPlugin`，由
  `rutis_host::languages::{node, python}::launcher` 构造，解释器、Python 路径和 stdio 沿用 Dim 现在的设置；
  标准错误经 `rutis_host::output::lines` 输出。
  - 使用配置路径时，`SessionNodeRuntime` 和 `SessionPythonRuntime` 这两行把各自的 `SpawnPlugin` 作为子插件
    挂载并等待它。loader 的行本身已经是一行，而 `SpawnPlugin` 会把自己的 fiber 标记为运行时的行，
    所以它不能直接跑在行的 fiber 上。
  - 在 `RuntimeManager` 下，由 `Mounted` 子插件持有 `SpawnPlugin`。`runtime.list` 读取
    `RuntimeHandle::state()`，`runtime.install` 重启这个子插件，与现在相同。
- **标识。** App Server 的根提供 `LocalId("dim-daemon")`。这个内核在每个会话里都用它问候，运行时的会话和
  leaf 的会话（M8）都是。它不挂载 `Session::service()`：那个插件提供名为 `sessions` 的服务，而这个名字已经是
  Dim 的 `AgentDirectory`。
- **解析。** 一个 `SessionResolver::npm(catalog, "session-node")` 服务两个运行时，放在解析链最后。
  - 对每个运行时调用 `runtime(handle)`；在 loader 挂载前调用 `attach(&loader)`；loader 运行后对每个运行时
    调用 `follow_row`。
  - Dim 自己的包装改名为 `SnapshotResolver`。它继续检查每一行的 `provides` 快照、加上 `workflowFiles`，
    但不再分流 `py:` 名字。
  - 识别"运行时没在运行"的方式与 loader 相同：没有 schema，且 `meta.schema` 是一段文字。不匹配这段文字的内容。
- **门槛。** 准入原来等 `RuntimeRows`，现在改为等 `SessionRows::key(name)`。删除 `NodeRows` 部件以及
  `SessionNodeRows` / `SessionPythonRows` 两行。
- **重新加载。** `plugin.reload` 只调用 `Loader::reload(row)`。没有解析器缓存了，Node 运行时每次解析都会重新
  描述模块。
- **Python。** 系统解释器的检查改为要求 3.12。

### M5. Dim 服务目录里的 `loader` 仍是仅限 Rust 的 `Loader`

0.19 的 `LoaderPlugin` 会提供标记服务 `loader`（行的 `create`、`reload`、`remove`）。Dim 的服务目录继续把
`loader` 登记为仅限 Rust 的 `Loader`，所以注入 `loader` 的外来行拿不到 rutis 的行服务。行仍然只由
PluginManager 写入（[PluginManager](plugins/PluginManager.md)）。`LoaderPlugin` 是平台插件，所以会话从不把
`loader` 导出给对端。Dim 没有任何行提供名为 `loader` 的服务。

### M6. loader 按宿主划分，Agent 建在宿主的 loader 里

一个宿主只有一个 loader。App Server 的 loader 属于它所在的进程，它的 Agent 都放进这个 loader
（`dim-app-server/src/agents.rs:1699`）。另外两种放置方式违反了这条规则：

- `build()` 给 Agent 一个私有 root 和一个自己的 loader。
- `build_in(ctx)` 给每个 Agent 在宿主下面再建一个自己的 loader，并按 Agent 隔离这个 loader 的服务，好让多个
  Agent 共用一个宿主（[single-tree](../../architecture/rust-agent/single-tree.md)、
  [Dim 如何使用 rutis §2](rutis-usage.md#2-插件是一行实例挂载它的部件)"隔离标签"）。

`build_in` 在 0.19 上行不通了。`LoaderPlugin` 还会提供 `LoaderService`，它是 rutis-loader 的私有类型，键无法隔离，
所以同一宿主里的第二个 loader 会因 `ServiceExists` 失败。本次升级恢复这条规则，Agent 只保留一种放置方式：

- **只有一个 API。** `AgentBuilder::build(loader, anchor)` 就是现在的 `build_at`，只是改名为 `build`。loader、Agent
  实例和 root 都归宿主所有，Agent 既不建 root 也不建 loader。App Server 只需要改名。
- **删除的内容。**
  - 会自建 loader 的 `build()`、`build_in`、`AgentHost::Private` 和 `AgentHost::Within`。
  - `own_placement` 按 Agent 对 `Loader`、`PartRegistry` 和 mounter 做的隔离，以及它的标签计数器。
  - `Agent::close` 只关闭 Agent 的 Session。Agent 实例由宿主卸载，root 由宿主关闭。
- **测试就是宿主。** 产品代码里没有别的放置方式。`build()` 和 `build_in` 的 156 处调用全部在测试和示例里（39 个文件）。
  - 在 `dim-agent-runtime` 现有的 `testing` feature 下提供一个测试宿主。它像 App Server 一样装配一个 root 和这个 root
    的 loader：builder 的模块和插件行、`agent_row`，以及一个 Agent 实例；再用 `build` 把 Agent 建在这个 loader 里，
    并负责 root 的生命周期。它以宿主的身份接手 `own_placement` 的装配，而不是 Agent 的一种放置方式。
  - 测试在 Agent 启动前挂上的宿主插件（`RecoveryHosts`、`AimuxPlugin`、`ConfiguredModelsPlugin` 等）改为通过
    `AgentBuilder::plugin` 成为行，或通过 `process_value`、`session_value` 作为值提供。
  - 测试在 Agent 运行期间挂上或卸下的宿主插件（`tests/integration/tests/tool_binding.rs` 里的 `EnableContributor`）
    改为在测试宿主的 loader 里增删行。
  - `agents_of_one_builder_mount_only_their_own_plugins_parts`（`tests/integration/tests/part_registry.rs`）改用两个
    测试宿主，并且仍然检查每个宿主都有自己的部件登记表。
- **文档。**
  - `AgentBuilder` 的文档和 single-tree 只写这一种放置方式。
  - Dim 如何使用 rutis §2 写明测试和 App Server 一样是宿主（"插件在哪里运行"），并删掉"两个共享宿主的 Agent 给各自
    的 loader、部件登记表和 mounter 用同一个标签"那句话（"隔离标签"）。

多个 Agent 仍然可以像在 App Server 里那样共享一个宿主，`agent_row` 已经会隔离进程值。M6 不依赖 0.19，因为
`build_at` 在 0.17.1 上就能用。它先行合入，作为一个单独的 pull request（§5）。

### M7. rutis 的输出不进入 TUI 的终端

- `dimcode` 的根改为 `Ctx::root_with_sink`，配一个什么都不写的 sink。
  - CLI 插件（`DaemonLink`、leaf、`Tui`）的失败，本来就在 CLI 等待它们的地方报告给 CLI。
  - 其他种类的错误，在 0.17.1 上也不会打印。
- App Server 保留它的 `tracing` sink。它的 loader 的配置警告通过 `Loader::on_config_warning` 转到 `tracing`。

### M8. UI leaf 是一个会话

leaf 保持 [Tui §3.6](../../design/plugins/Tui.md#36-ui-registry-and-ui-leaves) 的设计：daemon 把普通的 `Tui`
插件加载进交互进程，并负责卸载或替换它；生命周期和存活是 rutis 的事实；业务流量仍走协议 v3。变化的只是机制：

```text
daemon 根（LocalId dim-daemon）                   dimcode leaf 根
  UiHostPlugin                                     LeafNode
    └─ UiLeaf（每次 ui.open 一个）                   ├─ LocalId ui-<id>
         ├─ 对 catalog_key() 隔离的 ctx              ├─ catalog_key()：StaticCatalog { "Tui" }
         ├─ 拨号 address ──── 一条 channel ────────► └─ 只接受一次连接，pipe::framed
         ├─ 握手：对端 id 必须是 ui-<id>                  握手：对端 id 必须是 dim-daemon
         ├─ 会话 ◄═════════════ rutis 会话 ═════════════►  会话
         └─ TuiRow：run("Tui", {uiId, token,                 └─ Tui（被托管；由 dim-daemon 运行）
                    sessionId}) → loaded                          └─ DaemonLink ── 协议 v3 ──► Rpc
```

- **leaf 侧。** `LeafNode` 提供 `LocalId(ui-<id>)`，并在 `catalog_key()` 提供只含 `Tui` 的 `StaticCatalog`。
  它像现在的 `Accept` 一样，在私有地址上只接受一次连接（现在是一个返回 `Channel` 的普通函数），然后用
  `Session::handshake` 问候。对端 id 不是 `dim-daemon` 时直接丢弃，channel 随之关闭。之后 leaf 挂载这个会话，
  会话结束代替原来的 `LinkState::Stopped`。
- **daemon 侧。** `UiLeaf` 像现在的 `LeafDialer` 一样拨号到这个地址（现在是一个普通函数）。它从对
  `catalog_key()` 做了隔离的上下文发起问候；对端 id 不是 `ui.open` 给出的 `peer` 时直接丢弃。之后挂载会话。
- **`TuiRow`。** 它调用 `SessionHandle::run("Tui", config)`，并在 `RowHandle::loaded()` 时让 `ui.open` 完成。
  - leaf 的会话还不能运行插件时，`run` 会立即失败。`TuiRow` 在 `ui.open` 原有的启动期限内重试这种失败，
    也就是 §3.6 已经描述过的、等待 `plugins` offer 的那段等待。
  - 它的效果是丢弃这个行句柄，从而在现有的卸载时限内卸载 leaf 上的 `Tui`（终端恢复）。超时就关闭 channel
    （`Close::Done`）；会话在行之后关闭。
- **停止。** 两侧都用会话 fiber 的结束代替 `LinkState::Stopped`：daemon 上调用 `UiHost::remove`，leaf 上
  销毁客户端树。
- **版本不一致。** 链接协议发生了不兼容的变化，所以 `hello` 公布的 feature 从 `ui.leaf` 改为 `ui.leaf.2`。
  这样，无论是旧 CLI 连新 daemon，还是新 CLI 连旧 daemon，Tui §3.6 现有的回退机制都会改为直接运行 TUI。
- **链接现在承载的内容**（对 [Tui §8](../../design/plugins/Tui.md#8-security) 的更新）：
  - daemon 上由非平台插件提供的进程级服务（`sessions`、`settings`、`pluginFiles`、`auxiliaryModel`、
    `sessionMetadata`、`managementMethods`）对 leaf 可见。leaf 只能经由本地 `ui.open` 调用方给出的私有地址
    连通，被托管的 `Tui` 仍然只使用协议 v3。信任边界仍是本地用户，但 §8 中"leaf 链接不暴露任何 Session 或
    管理方法"一句改为这里的说法。
  - leaf 不能在 daemon 上运行任何插件：会话插件目录被隔离掉了。
  - daemon 只能在 leaf 上运行 `Tui`：leaf 的目录里没有别的。
  - `UiLeaf` 只拨号 loopback 的 `tcp:` 地址（Windows 上私有地址的形式）。
  - daemon 的监听器名字会公布给每个 leaf。leaf 上不发出任何事件，所以没有事件跨过链接。
- **将来的 UI 事件。** 重写 Tui §3.6 的"Later"一节：0.19 没有 `export`/`import` 闸门，也没有前缀订阅，
  所以 leaf 发出的 `ui/*` 事件会到达 daemon 上所有同名监听器。这些事件的契约（serde 载荷，带 `uiId` 和
  `sessionId`，只通过 adopted input 进入）不变，仍未实现。
- **接受的限制。** Tui §10 的两条限制不再存在：rutis 已经没有私有的 `peer:` 行，也没有本地传输。改为接受
  两条新的限制，而不向 rutis 提需求：会话插件目录的键是 `#[doc(hidden)]`，不过 rutis-bridge 的 catalog
  模块文档把它写成宿主提供目录的位置；会话没有"可以运行插件了"的公开就绪信号，所以 `TuiRow` 重试 `run`。

### M9. 较小的替换

- `plugin_manager.rs`：`waitingFor` 列出 `Loader::waits(entry)` 里的服务。等待运行时的行会写出那个运行时。
- 9 处 `set_overlay` 测试调用改为把夹具行写进一层自己的配置，再用它做 reconcile。
- `tests/integration/tests/extensions.rs` 用一个不走 npm 解析的名字保留"模块无法解析"的用例。裸的 npm 名字
  现在会等待运行时，而不是失败。
- `xtask types` 改用 `rutis_types::Exports`。为新的文件头重新生成一次 `npm/branch-services.d.ts`。
  `AgentErrorThrown` 保持原名：只有名为 `Error` 的错误类型才会改名。
- Dim 手写的 dispatcher 继续用 `session::Error::Value` 拒绝参数和方法。改用 `Binding` 错误码不在本次升级范围内。

### M10. TypeScript 插件测试运行在 `rutis-host` 上

- `runtime/package.json` 加开发依赖 `@arcships/rutis-host`。CI 通过 `RUTIS_HOST` 或 `PATH` 找到它。
- SessionTitle 的测试加载的是具名导出 `part`。这个部件移到单独的模块，`index.ts` 和测试都导入它的默认导出。
  其他插件的测试本来就加载默认导出。
- `load(plugin, { config, services })` 的形状不变。现在测试和插件之间传值遵循进程之间的规则，测试观察到的
  事件会在发出方返回之后才到达。在 `emit` 之后立刻读取结果的测试要改为等待。

## 5. 实施计划

M6 先行合入，作为一个单独的 pull request，基于 0.17.1。它不改变对 rutis 的任何用法，而它的测试迁移量如果并进来，
会淹没升级本身的 diff。之后握手不允许部分升级，所以步骤 1–5 放在一个 pull request 里。每一步是一个独立的提交。

| 步骤 | 工作 | 门槛 |
| --- | --- | --- |
| 0（单独的 pull request，基于 0.17.1） | M6：只保留一个放置 API，提供测试宿主，把 39 个测试和示例文件迁到它上面 | `cargo test --workspace --locked`；§6 中放置的检查 |
| 1 | 固定版本和锁文件（M1）；作者指南的固定版本 | `cargo metadata` 能解析；锁文件写的是 0.19.0 |
| 2 | 内核与服务目录：M2、M3 | `dim-app-server` 以下各 crate 和 `tests/integration` 的测试（`extensions.rs` 在步骤 3 跟上）；§6 中 Timeline `before` 的检查 |
| 3 | 运行时：M4、M5、M9 | App Server 和 `tests/integration` 中由 Node、Python 驱动的流程测试 |
| 4 | leaf：M7、M8 | `cargo test -p dimcode --test ui_leaf`；`transport_flow ui_host::` |
| 5 | TypeScript 包与工具：M10、`xtask types` | `runtime/plugins/*` 的插件测试；`runtime/plugin-sdk.test.mjs`；`xtask check` |
| 6 | 文档（§7） | `node scripts/adr-check.mjs`；`node --test scripts/*.test.mjs` |

## 6. 验证

在工作区根目录按 CI 顺序运行完整的 CI 序列：

1. `./dev check`
2. `cargo clippy --workspace --all-targets --locked -- -D warnings`
3. `cargo test --workspace --locked`
4. `cargo run --locked -p xtask -- check`，然后 `graph`。装配只因 M4 删除的行而变化；`.json` 快照的 diff
   只能出现这一项
5. `node --test scripts/*.test.mjs`
6. 按 CI 的方式运行 Node 绑定：`runtime/plugin-sdk.test.mjs`、插件测试、`tsc` 检查和
   `git diff --exit-code -- npm/branch-services.d.ts`
7. 由 Node 和 Python 驱动的 Rust 流程（`DIM_TEST_NODE`、`DIM_TEST_JS_RUNTIME`、`DIM_TEST_RUTIS_SDK`），以及
   `cargo test -p dimcode --test cli_flow -- --ignored`

测试套件必须覆盖的流程检查；只在契约变化处扩展现有测试：

- **`before` 顺序（M2）：** 同一分支上的两个变形按现在的顺序生效并记录各自的步骤；某个变形失败时保留原始 draft；
  另一个分支的变形不运行。
- **运行时的行（M4）：** TypeScript 行和 Python 行都能启动，`runtime.list` 能列出它们，经历一次
  `runtime.install` 重启后仍在，准入会等待运行时的行；`plugin.reload` 能用上改动后的模块。
- **快照行（M4）：** 运行时停着时解析的行，按它的 `provides` 快照被接受。
- **`loader`（M5）：** 注入 `loader` 的 TypeScript 行拿不到 rutis 的行服务。
- **放置（M6）：** 迁到测试宿主上的测试保留原有断言。同一个 builder 构建、放在两个测试宿主里的两个 Agent 只挂载
  各自插件的部件。Session 运行期间测试加入、随后又删除的贡献者行，会先进入 Session 的工具，之后再离开。关闭 Agent
  后它的实例留给宿主处理，关闭测试宿主的 root 时实例被卸载。
- **leaf（M8）：** 现有的 `ui_leaf` PTY 流程（启动、`/new`、`/web`、退出码、替换代次、被杀掉的 leaf、
  `ui.close`、daemon 关闭）。新增三项检查：
  - leaf 试图在 daemon 上运行插件时，因为没有安装该插件而失败；
  - 对端用别的 id 问候时，两侧都会拒绝；
  - `hello` 提供 `ui.leaf.2`，找不到它的客户端改为直接运行 TUI。
- **终端（M7）：** daemon 消失时，leaf 在 TUI 恢复终端之前不往终端打印任何内容。

## 7. 随升级一并更新的文档

- [Dim 如何使用 rutis](rutis-usage.md)：§1 和 §2（插件在哪里运行、隔离标签，M6）、§3.4（M3）、§3.5（运行时的行与 `isolate`）、
  §5（"Dim 使用 rutis 0.17.1"）、附录 A（`LocalId`、运行时的绑定）和附录 B（Python 3.12）。
- [Tui](../../design/plugins/Tui.md)：§3.1、§3.6（示意图、Link、Hosted `Tui`、Later）、§8、§9 和 §10（M8）。
- [RuntimeManager](plugins/RuntimeManager.md)：示意图、Python 版本要求、运行时与解析器各节，以及服务名（M4）。
- [PluginManager](plugins/PluginManager.md)：`waitingFor` 和重新加载（M9、M4）。
- [Session](../../design/session-typescript.md)、[分支](../../design/branch-typescript.md)和
  [进程](../../design/process-typescript.md)的 TypeScript 设计，[作者指南](../../typescript-plugin-authoring.md)，
  [single-tree](../../architecture/rust-agent/single-tree.md)（也包括 Agent 的放置方式，M6），[lifecycle](../../architecture/rust-agent/lifecycle.md)，
  [插件页面索引](../../architecture/rust-agent/plugins/README.md)和[架构概览](../../architecture/README.md)：
  其中对 0.17 的引用，以及运行时和 leaf 的机制。
- 已有中文版的，同时更新 `docs/zh/` 下的对应文档。`docs/architecture/rust-agent/generated/` 下的生成页面
  只通过 `xtask graph` 更新。

## 8. 风险

| 风险 | 处理 |
| --- | --- |
| 宿主、运行时和 leaf 之间版本不一致（握手会拒绝） | M1 在一个提交里升级所有固定版本；`ui.leaf.2` 让版本不一致的 CLI 和 daemon 改走直接 TUI |
| daemon 的服务变得可以从 leaf 访问 | M8：私有地址、隔离会话插件目录、核对 id、只拨 loopback；Tui §8 写明这一点 |
| 宿主里出现第二个 loader | M6：loader 按宿主划分，Agent 从不自建 loader；App Server 和每个测试宿主各持有一个 |
| 迁移 156 处测试调用改变了测试所检查的内容 | 每个迁出的测试保留原有断言；M6 单独基于 0.17.1 合入，它的 diff 里没有升级本身的改动 |
| rutis 往 TUI 的终端打印内容 | M7 的 sink；rutis-bridge 在会话无法运行插件时自己调用的 `eprintln!`，由终端检查覆盖 |
| 配置不符合 schema 的 TypeScript 插件现在会失败 | 插件测试和 Node 驱动的流程使用随产品发布的配置运行 |
| 不认识的 npm 名字会一直等待，而不是失败 | `RuntimeManager` 的状态会显示这个等待；M9 调整依赖原有失败行为的测试 |
| 经由 `rutis-host` 会编译两个版本的 tokio-tungstenite（0.29 和 0.30） | 接受；Dim 自己的 WebSocket 代码继续使用 0.29 |

回滚：撤销这次升级改动和固定版本即可。没有任何持久化格式变化；Timeline、存储和线上格式的夹具都不动。

## 9. 向机制所有者提出的需求

无。每项决策都按 rutis 0.19 的现状使用它。接受两条限制而不提需求（M8）。`LoaderService` 的键无法隔离，
这正好符合 loader 按宿主划分（M6）。

## 10. 规则检查

本次升级不新增部件、事件或事实。

| 规则 | 回答 |
| --- | --- |
| 原则 §4 规则 1–3、5–7、9 | 不变：命令、交接、`before` 契约、批次、上下文路径和事实都不变；M2 保留 `before` 的契约和顺序 |
| §4 规则 4（状态归属） | 行仍归 PluginManager（M5）；运行时状态仍归 `RuntimeManager`，从 rutis 的句柄读取 |
| §4 规则 8（基础设施默认不变） | 改动来自依赖升级，限定在 runtime crate、App Server 和 CLI 之内；不向 rutis 提需求 |
| §4 规则 11（受信插件，明确的边界） | M8 写明 leaf 链接暴露了什么，信任边界保持在本地用户；审批和权限都不变 |
| AGENTS 规则 6（冻结的夹具） | 不动 `crates/dim-agent-types/tests/fixtures/` |
| ADR 002 D1–D5 | 放置、接线、依赖和配置都不变；运行时仍是进程部件，leaf 仍属于 CLI 进程 |
| ADR 006 D2 | Dim 直接使用 rutis 的会话、解析器和运行时插件，不为被删的 API 加包装 |

## 11. 消融检查

- 直接删除、不做替代：`NodeRows` 及其两行；解析器缓存和 `invalidate`；`share_registered_only`；`TuiRow` 对
  `plugins` offer 的等待；leaf 和 daemon 各自的 `Transport` 实现、`StaticIdentity`，以及 `UiHostPlugin` 的传输和身份；
  会自建 loader 的 `build()`、`build_in`、`AgentHost::Private` 和 `Within`，以及 `own_placement` 按 Agent 的隔离及其
  标签计数器（M6）。
- `own_placement` 的装配移到测试宿主里，那是唯一需要最小宿主的地方。它在那里是一个宿主，而不是 Agent 的一种放置方式（M6）。
- Dim 只为 rutis 不做的事保留自己的解析器包装：`provides` 快照检查和 `workflowFiles`。
- 没有兼容层：直接使用 `rutis-host` 的 `SpawnPlugin`，而不是复制一份。复制等于在 Dim 里重建被删的 `LocalRuntime`。
- `ui.leaf.2` 复用现有的 feature 回退机制，不新增版本检查。
