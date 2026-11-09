# 服务面：跨语言形状的策略、盘点与第一批改造

状态：Draft。对应 [#1289](https://github.com/arcships/dim/issues/1289)（策略）、[#1300](https://github.com/arcships/dim/issues/1300)（盘点）、[#1292](https://github.com/arcships/dim/issues/1292)（第一批）。父 issue [#1288](https://github.com/arcships/dim/issues/1288)。研究背景见 `docs/design/research/remote-plugins.md`（由 [#1299](https://github.com/arcships/dim/pull/1299) 引入）。

本文是改动设计，不是实现记录。所有 API 都是目标形状；现有代码位置用于说明改哪里。

## 1. 规则

1. **数据面是默认**。公开服务的方法是 serde 数据：参数、返回值、错误都是普通类型；不出现句柄、`Arc<dyn Trait>`、`&mut`、迭代器、通道、`PathBuf`/`OsString`。
2. **身份是值**。Session、branch、task、file、root、subscription 都是 id；id 不是授权。
3. **操作代替对象**。进程、PTY、文件、store 都是一组操作加 id；对象和句柄留在所有者内部。
4. **流是事件或订阅**。大输出分页；回调可以按引用跨语言，但不在 commit 锁里同步调用。
5. **错误是数据**。`{code, message}`；转发方原样抛。
6. **取消是调用**。显式 cancel，或调用方丢弃。

`internal` **不是“写个原因”**，而是：这个服务面不能被另一个语言或进程实现、提供或注入。它不能借给 leaf，不能由 leaf 提供，不能跨作用域注入。原因解释为什么这个面不能数据化（状态机、锁内回调、OS 对象、secret、热路径）。功能可以在 leaf 里重新实现，但那是新的数据服务；同一进程内换一个同类型契约的 Rust 实现不算跨语言替换。作用域与它无关：instance-scoped 的 `timeline`/`dispatcher` 是数据，process-scoped 的 `hostPlatform` 是 `internal`。

## 2. 策略怎么落地（#1289）

### 2.1 标记

**数据面 = 名字在 catalog 里 shared，并且在 `host_key(name)` 上有一个 `HostDispatch`。** 两个条件缺一不可。`#[rutis::service]`、`register_shared`、`register_shared_instance` 都是标记写法，但 shared 本身不是数据面：`localId`、`loaderService`、`pluginMounter` 今天是 shared 却没有 dispatcher，leaf 不能调用它们。

`internal` 需要一条原因，放在 `AgentBuilder` 的旁表里，不改 rutis 的 `ServiceCatalog`：

```rust
pub struct Faces {
    reasons: HashMap<String, &'static str>,
}

impl AgentBuilder {
    /// Marks the name of T as internal, with the reason a face cannot cross.
    pub fn internal<T: ?Sized + 'static>(&mut self, reason: &'static str);
    /// The same for a name that has no Rust type (a hand-written dispatcher).
    pub fn internal_name(&mut self, name: &str, reason: &'static str);
    pub fn faces(&self) -> &Faces;
}
```

各 crate 的 `register_names(builder: &mut AgentBuilder)` 在注册完名字后，对自己不共享的名字调用 `builder.internal::<T>(reason)`。`xtask` 和 App Server 用的是同一个 builder，所以 reason 只有一份。

### 2.2 catalog 与检查

- `xtask graph` 把 `Crosses languages` 列换成 `face`：
  - shared 且有 dispatcher：`data`；
  - 非 shared 且有 reason：`internal(<reason>)`；
  - shared 但没有 dispatcher：`shared-no-dispatcher`，生成失败；
  - 非 shared 且没有 reason：生成失败，报名字。
- `xtask check` 双向检查：
  - 一个在 `host_key(name)` 上有 dispatcher、但没有 shared 的名字失败（`managementMethods` 是这一类）；
  - 一个 shared、但没有 dispatcher 的名字失败（`localId`、`loaderService`、`pluginMounter` 是这一类）；
  - 一个名字同时 shared 和有 reason 是矛盾，失败。
- `managementMethods` 不是只加一行：`session_typescript.rs:80` 已经 `register_shared`，但 `xtask/src/catalog.rs:81` 在 `serve` 注册 typed 名字之前调用 `share_names`，后者会被覆盖。改动是让 shared 注册发生在 typed 注册之后，或让 `register_names` 保留已有的 shared；这是 #1289 的代码项。
- `localId`、`loaderService`、`pluginMounter` 三个 shared-without-dispatcher 在 #1289 里逐个解决：需要 leaf 的补一个 `get`/`status` dispatcher，不需要的停止 shared 并写 reason。检查在解决前会失败，这是有意的。
- `xtask/src/catalog.rs` 的 `crosses` 字段改名 `face`，枚举 `Data | Internal(&'static str) | SharedNoDispatcher`。

### 2.3 leaf 的运行时拒绝

- `PartRegistry` 挂载 leaf 部件前，`Names` 解析每个 `inject` 名字：`face != data` → 失败，错误码 `internal_service`，消息写明名字和原因。
- 一个 leaf 行声明 `provides` 的名字时，同样要求 data；否则它的服务在宿主侧不可解析，直接拒绝，不等到第一次调用。
- `Names` 需要读到 `Faces`：新增 `Names::copy_with_faces(catalog, build, faces)`；`Names::copy` 保持原签名并传空表。不要给 `copy` 加可选参数（Rust 没有），也不要改所有调用点。

### 2.4 作者侧与原因词汇

- 新服务默认写数据面；有 leaf/跨语言消费者时不需要另写适配层。
- 只有确实无法数据化的面才标 `internal`，原因必须是闭集里的一个：
  - `object`：句柄/OS 对象，需要拆操作；
  - `hot-path`：每回合/每条输入的状态机或严格本地顺序；
  - `lock-callback`：commit 锁内同步回调；
  - `secret`：凭据/token，只暴露操作；
  - `owner-api`：类型化所有者 API，已有数据兄弟，或只给本地用；
  - `control`：宿主入口/控制面；
  - `host-resource`：FS 索引、活进程等宿主资源；
  - `legacy-no-consumer`：内容是数据，但还没有 leaf 消费者。
- 模板和 PR 清单固定一问：“公开服务的方法只有数据；句柄留在哪一层内部？”

### 2.5 存量迁移

存量不一次改完。每个非 shared 的名字先标 `internal("legacy: <category>; see #1300")`，等有 leaf 消费者时改数据面；#1300 的盘点表是清单。这样 `xtask check` 从第一天就能拦住“新写的服务忘了形状”，而不用等 84 个名字全部迁完。

## 3. 盘点（#1300）

catalog 现在 109 个名字：25 shared、84 Rust-only。§3 盘点这 84 个现有名字，并在 §3.2 标出 3 个新名字（`scheduleJobs`、`workflowRuns`、`managedSearch`）；新名字不计入 84。每个现有名字恰好出现一次。

### 3.1 已经可以跨，但 catalog 列没体现

| 名字 | 处理 |
| --- | --- |
| `managementMethods` | 在 `host_key` 上有 dispatcher；修 shared 注册顺序后 catalog 变 `data`。它是 #1289 的代码项，不是迁移。 |

### 3.2 第一批数据面（#1292）

| 名字 | 目标 | 备注 |
| --- | --- | --- |
| `scheduleJobs` | 新数据面 `scheduleJobs.call(JobRequest)` | 按 [Scheduler](plugins/Scheduler.md) 已接受的 `JobRequest`/`Caller`/`JobReply`；它们现在没有 serde derive，随迁移补 |
| `workflowRuns` | 新数据面 `workflowRuns.call(RunRequest)` | 按 [Workflow](plugins/Workflow.md) 已接受的 `RunRequest`/`RunReply`；Workflow 未实现，接口预留 |
| `sessionFilesHost` | 现有名字改 shared + dispatcher | 路径用字符串；不新增 `sessionFiles` |
| `managedSearch` | 新数据面，只含 `search` | `managedProviders` 保持 internal(secret)，账号/token 不出去 |

`settingsService` 和 `modelHost` **不进第一批**：它们保留类型化调用。`settings.read` 会新建取消 token、并返回 `SettingsResult` 信封；GoalBranch 需要自己的 stop token，改成数据面会丢取消。`auxiliaryModel.complete` 没有 tools、config 和 cached prefix；PromptSuggestion 克隆的是整轮 `ModelRequest`，换过去是行为变化，不只是形状。它们已有数据兄弟给其他消费者；有 leaf 消费者时再按 §4 的规则扩展。

### 3.3 所有者 API，已有数据兄弟

| 名字 | 数据兄弟 | 说明 |
| --- | --- | --- |
| `agentDirectory`、`agentIndex` | `sessions` | 目录查询走 `sessions` |
| `settingsService` | `settings` | GoalBranch 保留 typed；`settings` 给其他消费者 |
| `modelHost` | `auxiliaryModel` | PromptSuggestion 保留 typed；`auxiliaryModel` 给无 tools 的完成 |
| `auxiliaryModelHost` | `auxiliaryModel` | 模型调用走 `complete` |
| `loader` | `rutis/loader` | 插件管理走数据面 |
| `sessionStore` | `sessionMetadata`、`sessionFilesHost` | 行与文件分开 |
| `timelineTree`、`timelineTreeControl` | `treeCommands` | 建/读分支走 `treeCommands` |
| `toolSourcesView` | `toolRegistry` | 工具贡献走注册表 |
| `callers` | `callChannel` | 调用通道走数据面 |

### 3.4 对象/句柄：拆成操作加 id

| 名字 | 现在的对象 | 目标操作 |
| --- | --- | --- |
| `hostPlatform` | `ProcessTree`、`PtyProcessTree`、临时目录 | `start`/`wait`/`kill`/`output`/`shell`/`write`/`resize` |
| `fileStorage` | `FilePath`、根与代次 | `open`/`read`/`write`/`close`/`list`/`remove` |
| `sqliteStorage` | 连接与事务 | store 端口（#1298） |
| `timelineStore` | 连接与批次 | `append`/`query`/`ack`（#1298） |
| `toolHost` | `ToolDriver` | `execute` 数据面 |
| `mcpSession` | 会话对象 | 每 server 的 id 操作 |
| `backgroundExec` | 任务对象 | task id 操作 |
| `approval`、`permission` | 决策状态 | `decide` 数据面 |
| `inputQueue` | 队列状态 | `submit`/`withdraw` 数据面 |

这些面即使有数据接口，实例核心仍默认留在宿主；它们列在这里是为了说明“对象形状”不是永久 internal，而是需要操作化；原因 `object`。

### 3.5 热路径与顺序：契约上 internal

`agentLoop`、`loopState`、`modelProvider`、`prompt`、`contextPreparation`、`toolBinding`、`toolExecutor`、`branchOpening`、`configurationPublisher`、`configureControl`、`fileReads`、`branchLine`、`wait`、`submitChecks`、`commandRulesHost`、`promptHost`、`promptRegistry`、`initialPromptConfiguration`、`historyRenderer`、`contextModelHost`、`prepareModelHost`、`prepareContextHost`、`compactionStrategyHost`、`faultReporter`。

原因 `hot-path`：每回合/每条输入的状态机、严格本地顺序，或同步回调。跨了要重写顺序和失败语义，收益不成比例。

### 3.6 入口、secret 与宿主资源

`serverControl`、`connectionHub`、`pluginFileTransactions`、`publication`、`mcpAuth`、`grokOAuthService`、`openAICodexOAuthService`、`uiHost`、`uiExtensions`、`mentionIndex`、`skillsCatalog`、`mcpService`、`hooksService`、`providerService`、`providerConnectionResolver`、`collaboration`、`sessionLoader`、`timelineFactory`、`sessionSkills`、`skillsBranch`、`managedProviders`。

逐项原因：`serverControl`、`connectionHub`、`uiHost`、`uiExtensions`、`sessionLoader`、`timelineFactory` 是 `control`；`mcpAuth`、`grokOAuthService`、`openAICodexOAuthService`、`providerService`、`managedProviders` 是 `secret`；`pluginFileTransactions`、`publication`、`mentionIndex`、`skillsCatalog`、`mcpService`、`hooksService`、`providerConnectionResolver`、`collaboration`、`sessionSkills`、`skillsBranch` 是 `host-resource`。`managedProviders` 的搜索走新的 `managedSearch`；账号与 token 仍留在这个 internal 名字上。

### 3.7 数据内容，但还没有 leaf 消费者

`executionSettingsHost`、`toolchainSourceHost`、`promptSuggestionSettingsHost`、`selectableModelsHost`、`modelSelectionHost`、`modalityConnectionHost`、`worktreeAssociations`、`listenAddresses`、`credentialVariablesHost`、`reservedMethods`、`agentConfig`、`collaborationConfig`、`skillsConfig`、`collaborationDirectory`、`sessionClientsHost`、`collaborationSession`。

原因 `legacy-no-consumer`：内容是数据，但还没有 leaf 消费者。它们不是“永远不能跨”。

## 4. 第一批改造（#1292）

每个服务一个 PR；数据面委托现有实现，不写第二份业务逻辑。测试在现有 flow 上扩展，并加一个 loopback session 的跨边界用例。

### 4.1 `settingsService`：不迁移

`GoalBranch` 保留类型化 `SettingsService`。`settings.read` 自己建取消 token、返回 `SettingsResult<T>` 信封（`{ok:true,value}` / `{ok:false,error}`）；换成数据面会丢掉分支的 stop token，也要解一层信封。`settings` 已经是其他 leaf 可用的数据面，不需要为 GoalBranch 再改。

测试：Goal 现有的设置读取与上限测试保持；不为这一项加 loopback。

### 4.2 `modelHost`：不迁移

`PromptSuggestion` 保留 `ModelHost` 和 `ProviderConnectionResolver`。它克隆整轮 `ModelRequest`（provider、model、system、tools、config、cached prefix）并流式读取；`auxiliaryModel.complete` 没有 tools/config/prefix，换过去会静默改变行为。它的设置是 `enabled`/`maxChars`（`prompt_suggestion.rs:61-63`），模型来自当轮请求，不是设置；文中不再写成 `enabled`/`model`。

如果将来要让 PromptSuggestion 进 leaf，先扩展数据面承载完整请求、tools、config、取消，并加覆盖 tools/config 的测试；不要只换名字。

### 4.3 `scheduleJobs`

按 [Scheduler](plugins/Scheduler.md) 已接受的形状，不在这里重定义：

```text
scheduleJobs.call(JobRequest)
  JobRequest = {action, jobId?, job?, enabled?, limit?, caller: Caller}
  Caller = {sessionId, branchId, cwd}
  -> JobReply
  -> error {code, message}
```

改动：

- 给 `ScheduleJobs` 加 `#[rutis::service]` 数据面（或手写 dispatcher），在 `host_key("scheduleJobs")` 提供；本地仍用 typed。
- `ToolRequest`、`Caller`、`JobReply` 补 serde derive；字段按 Scheduler 附录，不另起一套。
- `SchedulerSession` 改注入 `dyn HostDispatch`；调用传 `caller` 和取消。
- 测试：Scheduler 的 Session 工具 flow；离线/取消。

### 4.4 `workflowRuns`

按 [Workflow](plugins/Workflow.md) 已接受的形状：

```text
workflowRuns.call(RunRequest)
  -> RunReply
  -> error {code, message}
```

Workflow 尚未实现；本项只保留接口，不写半成品。WorkflowSession 的 `toolRegistry` 注册和工具形状不变。

### 4.5 `sessionFilesHost`

把现有 `sessionFilesHost` 改成 shared + dispatcher，不新增 `sessionFiles` 名字。它已经几乎全是数据（`Vec<u8>`、`String`、标量），只有路径是 `PathBuf`。

```text
sessionFilesHost.put {sessionId, name, content: bytes, cancel?}
  -> {fileId, path, bytes}
sessionFilesHost.register {sessionId, path, cancel?}
sessionFilesHost.owns {sessionId, path} -> {owned: bool}
sessionFilesHost.ownedPaths {sessionId} -> {paths: [string]}
  -> error {code: not_found | invalid | too_large | node_unavailable | cancelled | storage, message}
```

- 路径是 UTF-8 字符串；`content` 用 `rutis_bridge::session::Bytes`。16 MiB 上限不变；先测是否超过 Channel 单条消息上限，超了加 `putBegin`/`putAppend`/`putCommit`，不偷偷截断。
- 取消：调用方丢弃调用，或传 `cancel` 回调；方法签名写明。
- 消费者：Attachments（`put`）、WorkspaceTools/ShellSession（`register`）、FileAccessFacts（`owns`/`ownedPaths`）。
- 测试：`session_files_flow.rs`、`attachments_flow.rs`，加 loopback。

### 4.6 `managedSearch`

不把 `managedProviders` 变成 shared；新增一个名字，只含搜索：

```text
managedSearch.search {query, count, cancel?}
  -> {results: [{title, url, snippet, publishedAt?}]}
  -> error {code: unavailable | search_failed, message}
```

- `managedProviders` 保持 internal(secret)：账号、登录、token、租约仍在类型化面。
- Web 的搜索改调 `managedSearch`；Web 的 settings 仍走 `settings`。
- 新名字使 catalog 多一行；它不在 84 个现有名字里。

## 5. 验证

- `xtask check`：双向测试（dispatcher 未 shared 失败；shared 无 dispatcher 失败；shared+reason 失败）。
- `xtask graph`：catalog 的 `face` 列与真实装配一致；`catalog.md` 的 diff 作为评审材料。
- 每个服务：现有 flow 测试 + 一个 loopback session 的跨边界用例；不新增 mock-only 测试。
- 数量：不改 `cargo test` 的总量要求；按受影响 crate 跑，最终 `cargo test --workspace --locked`。
- 改装配后跑 `cargo run --locked -p xtask -- check` 与 `graph`。

## 6. 风险与开放问题

1. **Channel 单条消息上限**：`sessionFilesHost.put` 的 16 MiB 可能超过限制。先测；超了就分块，不降低上限。
2. **secret**：`providerService`、`mcpAuth`、OAuth 服务、`managedProviders` 保持 internal；数据面只暴露操作，不暴露值。
3. **Workflow 未实现**：`workflowRuns` 是接口预留，不阻塞其余项。
4. **`Faces` 的传播**：`Names`、`PartRegistry`、`SessionResolver` 都要拿到同一份原因表；独立 host / 测试用空表，检查只在装配和 leaf 加载处生效。
5. **legacy 标记**：84 个名字里 16 个是“数据内容、无消费者”，先标 `legacy-no-consumer`；不是永久 internal。
6. **shared 与 dispatcher 不一致**：`managementMethods`、`localId`、`loaderService`、`pluginMounter` 证明 shared 不等于数据面；#1289 必须把这三类逐个解决，检查双向失败。

## 7. 交付顺序

1. **#1289**：`Faces`、`AgentBuilder::internal`、catalog `face`、双向 `xtask check`、leaf 拒绝、`managementMethods` 注册顺序、`localId`/`loaderService`/`pluginMounter` 的 dispatcher 或去 shared、模板/清单。此 PR 只加检查和标记，不改服务行为。
2. **#1300**：按 §3 的分组在 catalog 里补齐 reason；`xtask graph` 的 diff 就是盘点结果。此 PR 不改服务。
3. **#1292**：按 `scheduleJobs` → `sessionFilesHost` → `managedSearch` 的顺序，一项一个 PR；`workflowRuns` 随 Workflow 落地。`settingsService`、`modelHost` 不迁移。
4. 每项都带现有 flow 测试；不新增适配层，不写第二份逻辑。
