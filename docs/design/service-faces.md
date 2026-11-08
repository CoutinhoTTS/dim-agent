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

数据面不新增属性：**一个名字被 catalog 注册为 shared，并且装配在 `host_key(name)` 上提供 dispatcher，它就是数据面**。`#[rutis::service]`、`register_shared`、`register_shared_instance` 都是这个标记的写法。

`internal` 需要一条原因。原因放在 `AgentBuilder` 的旁表里，不改 rutis 的 `ServiceCatalog`：

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
  - 名字是 shared：`data`；
  - 名字不是 shared 且有 reason：`internal(<reason>)`；
  - 名字不是 shared 且没有 reason：生成失败，报名字。
- `xtask check`：
  - assembly 里每个被 provided 的进程/Session/branch 名字，必须在 catalog 里是 `data` 或 `internal(reason)`；
  - 一个名字同时 shared 和有 reason 是矛盾，失败；
  - 一个在 `host_key(name)` 上有 dispatcher、但没有注册 shared 的名字失败（`managementMethods` 属于这一类，必须改成 shared）。
- 现有的 `catalog.md` 的 “Crosses languages” 说明文字同步改成 `face`；`xtask/src/catalog.rs` 的 `crosses` 字段改名 `face`，枚举 `Data | Internal(&'static str)`。

### 2.3 leaf 的运行时拒绝

宿主给 leaf 借 key 的地方统一检查：

- `PartRegistry` 挂载 leaf 部件前，`Names` 解析每个 `inject` 名字：不是 shared → 失败，错误码 `internal_service`，消息写明名字和原因。
- 一个 leaf 行声明 `provides` 的名字时，同样要求 shared；否则它的服务在宿主侧不可解析，直接拒绝，不等到第一次调用。
- `Names` 需要能读到 `Faces`；`Names::copy` 增加一个可选 `Faces`（测试和独立 host 用空表）。

### 2.4 作者侧

- 新服务默认写数据面；有 leaf/跨语言消费者时不需要另写适配层。
- 只有确实无法数据化的面才标 `internal`，并写清原因类别：`object`、`hot-path`、`lock-callback`、`secret`、`owner-api`、`legacy-no-consumer`。
- 模板和 PR 清单固定一问：“公开服务的方法只有数据；句柄留在哪一层内部？”

### 2.5 存量迁移

存量不一次改完。每个非 shared 的名字先标 `internal("legacy: <category>; see #1300")`，等有 leaf 消费者时改数据面；#1300 的盘点表是清单。这样 `xtask check` 从第一天就能拦住“新写的服务忘了形状”，而不用等 84 个名字全部迁完。

## 3. 盘点（#1300）

catalog 共 109 个名字，25 个 shared（数据面），84 个 Rust-only。84 个按决定分组如下；每个名字恰好出现一次。

### 3.1 已经可以跨，但 catalog 列没体现

| 名字 | 处理 |
| --- | --- |
| `managementMethods` | 在 `host_key` 上有 dispatcher；把它注册为 shared，catalog 变 `data`。它是 #1289 的检查项，不是改造项。 |

### 3.2 第一批数据面（#1292）

| 名字 | 目标 | 备注 |
| --- | --- | --- |
| `settingsService` | 用 `settings` 数据服务 | GoalBranch 已经只读 declared source，改调用即可 |
| `modelHost` | 用 `auxiliaryModel` | PromptSuggestion 改成非流式完成 |
| `scheduleJobs` | 新数据面 `scheduleJobs` | `ToolRequest`/`Caller`/`JobReply` 已是数据 |
| `workflowRuns` | 新数据面 `workflowRuns` | Workflow 尚未实现；接口先定，随插件落地 |
| `sessionFilesHost` | 新数据面 `sessionFiles` | `put`/`register`/`owns`/`ownedPaths` |
| `managedProviders` | 新数据面 `managedProviders.search` | 只有 Web 的搜索需要；账号操作仍 internal |

### 3.3 所有者 API，已有数据兄弟

| 名字 | 数据兄弟 | 说明 |
| --- | --- | --- |
| `agentDirectory`、`agentIndex` | `sessions` | 目录查询走 `sessions` |
| `auxiliaryModelHost` | `auxiliaryModel` | 模型调用走 `complete` |
| `loader` | `rutis/loader` | 插件管理走数据面 |
| `sessionStore` | `sessionMetadata`、`sessionFiles` | 行与文件分开 |
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

这些面即使有数据接口，实例核心仍默认留在宿主；它们列在这里是为了说明“对象形状”不是永久 internal，而是需要操作化。

### 3.5 热路径与顺序：契约上 internal

`agentLoop`、`loopState`、`modelProvider`、`prompt`、`contextPreparation`、`toolBinding`、`toolExecutor`、`branchOpening`、`configurationPublisher`、`configureControl`、`fileReads`、`branchLine`、`wait`、`submitChecks`、`commandRulesHost`、`promptHost`、`promptRegistry`、`initialPromptConfiguration`、`historyRenderer`、`contextModelHost`、`prepareModelHost`、`prepareContextHost`、`compactionStrategyHost`、`faultReporter`。

原因：每回合/每条输入的状态机、严格本地顺序，或同步回调。跨了要重写顺序和失败语义，收益不成比例；标 `internal(hot-path)`。

### 3.6 入口、secret 与宿主资源

`serverControl`、`connectionHub`、`pluginFileTransactions`、`publication`、`mcpAuth`、`grokOAuthService`、`openAICodexOAuthService`、`uiHost`、`uiExtensions`、`mentionIndex`、`skillsCatalog`、`mcpService`、`hooksService`、`providerService`、`providerConnectionResolver`、`collaboration`、`sessionLoader`、`timelineFactory`、`sessionSkills`、`skillsBranch`。

原因：宿主入口、明文 secret、FS 索引，或活的进程状态。`internal(control|secret|host-resource)`。`managedProviders` 的搜索在 #1292 单独放出去；账号与 token 仍在这里。

### 3.7 数据内容，但还没有 leaf 消费者

`executionSettingsHost`、`toolchainSourceHost`、`promptSuggestionSettingsHost`、`selectableModelsHost`、`modelSelectionHost`、`modalityConnectionHost`、`worktreeAssociations`、`listenAddresses`、`credentialVariablesHost`、`reservedMethods`、`agentConfig`、`collaborationConfig`、`skillsConfig`、`collaborationDirectory`、`sessionClientsHost`、`collaborationSession`。

原因：内容是数据，但没有 leaf 消费者；按 #1300 的规则，先标 `internal(legacy-no-consumer)`，出现消费者时加数据面。它们不是“永远不能跨”。

## 4. 第一批改造（#1292）

每个服务一个 PR；数据面委托现有实现，不写第二份业务逻辑。所有测试都在现有 flow 测试上扩展，并加一个 loopback session 的跨边界用例。

### 4.1 `settingsService` → `settings`

现状：`GoalBranch` 注入 `SettingsService`，只调用 `read_declared("goal", User, cancel)`；`settings` 数据服务已经有 `read`，实现同一个 `read_declared`。

```text
settings.read {source: "goal", scope: "user"}
  -> {revision, document}
  -> error {code, message}
```

改动：

- `dim-goal/src/branch.rs` 改为注入 `dyn HostDispatch`（`names.named("settings")`），调用 `read`，把 `document` 反序列化成现有 `Document`。
- `SettingsService` 保持 `internal(owner-api)`；不做第二套读取。
- 测试：`dim-goal` 的设置读取与上限测试；loopback leaf 读取。

### 4.2 `modelHost` → `auxiliaryModel`

现状：`PromptSuggestion` 注入 `ModelHost`、`ProviderConnectionResolver`，用 `invoke` 拿流。

```text
auxiliaryModel.complete {
  model: {providerId, modelId},
  request: {system?, messages, maxOutputTokens?}
} -> {text, stopReason, usage?}
  -> error {code: invalid_request | model_unavailable | failed | cancelled}
```

改动：

- `PromptSuggestion` 用自己已有的设置/模型选择得到 `ModelId`，改调 `auxiliaryModel.complete`；建议是一次非流式完成，不需要 `ModelStream`。
- 它的自身设置（`prompt-suggestion` source，`enabled`/`model`）改从 `settings.read` 读；`promptSuggestionSettingsHost` 保持 `internal(owner-api)`。否则 leaf 里仍缺一个 Rust-only 依赖。
- 去掉 `ModelHost`、`ProviderConnectionResolver` 两个注入。`auxiliaryModel` 的所有者仍解析连接与凭据，secret 不出宿主。
- 测试：建议生成 flow；模型不可用/取消的错误映射；loopback leaf。

### 4.3 `scheduleJobs`

现状：`SchedulerSession` 注入 `ScheduleJobs`，调用 `call(ToolRequest, Caller)`。

```text
scheduleJobs.call {request: ToolRequest, caller: {sessionId, branchId, cwd}}
  -> {content, records}
  -> error {code, message}
```

改动：

- 给 `ScheduleJobs` 加 `#[rutis::service]` 数据面（或手写 dispatcher），在 `host_key("scheduleJobs")` 提供；本地仍用 typed。
- `SchedulerSession` 改注入 `dyn HostDispatch`；`Caller` 用 serde 字段。
- 测试：Scheduler 的 Session 工具 flow；离线/取消。

### 4.4 `workflowRuns`

现状：Workflow 尚未实现；`workflowRuns` 只有设计里的名字。

```text
workflowRuns {action: "list" | "start" | "status" | "stop",
              name?, runId?, params?}
  -> {runs? | run?}
  -> error {code, message}
```

改动：接口随 Workflow 插件一起落地；在此之前 #1292 的这一项保持未开始，不写半成品。WorkflowSession 的 `toolRegistry` 注册和工具形状不变。

### 4.5 `sessionFiles`

现状：`SessionFilesService` 提供 `put`/`register`/`owns`/`ownedPaths`，返回 `StoredSessionFile {file_id, path: PathBuf, bytes}`。

```text
sessionFiles.put {sessionId, name, content: bytes}
  -> {fileId, path, bytes}
sessionFiles.register {sessionId, path}
sessionFiles.owns {sessionId, path} -> {owned: bool}
sessionFiles.ownedPaths {sessionId} -> {paths: [string]}
  -> error {code: not_found | invalid | too_large | node_unavailable | cancelled | storage, message}
```

改动：

- 新数据名 `sessionFiles`：在 `host_key("sessionFiles")` 提供 dispatcher，实现委托给现有 `SessionFilesHost`；`sessionFilesHost` 保持 typed、`internal(owner-api)`，给本地消费者。
- 路径是 UTF-8 字符串；`content` 用 `rutis_bridge::session::Bytes`。16 MiB 上限不变；先测一次 16 MiB 是否超过 Channel 的单条消息上限，超了就加 `putBegin`/`putAppend`/`putCommit`，不偷偷截断。
- 消费者：Attachments（`put`）、WorkspaceTools/ShellSession（`register`）、FileAccessFacts（`owns`/`ownedPaths`）。
- 测试：`session_files_flow.rs`、`attachments_flow.rs`，加 loopback。

### 4.6 `managedProviders`

现状：Web 注入 `ManagedProviders`，只调用 `search(WebSearchRequest) -> WebSearchResponse`。

```text
managedProviders.search {query, count}
  -> {results: [{title, url, snippet, publishedAt?}]}
  -> error {code: unavailable | search_failed, message}
```

改动：

- 只把 `search` 做成数据面；账号、登录、token、租约等仍 `internal(secret)`。
- Web 同时依赖 `SettingsService`（读它的 declared source）。要 leaf 化 Web，`settings` 数据面足够；否则 Web 暂时留宿主，只把 `search` 数据面备好。
- 测试：Web 搜索 flow；无可用服务时的 `search_unavailable`。

## 5. 验证

- `xtask check`：新检查项本身要有测试（一个未标记的 provided 名字失败；一个 shared+reason 失败；一个 `host_key` dispatcher 未 shared 失败）。
- `xtask graph`：catalog 的 `face` 列与真实装配一致；`catalog.md` 的 diff 作为评审材料。
- 每个服务：现有 flow 测试 + 一个 loopback session 的跨边界用例；不新增 mock-only 测试。
- 数量：不改 `cargo test` 的总量要求；按受影响 crate 跑，最终 `cargo test --workspace --locked`。
- 改装配后跑 `cargo run --locked -p xtask -- check` 与 `graph`。

## 6. 风险与开放问题

1. **Channel 单条消息上限**：`sessionFiles.put` 的 16 MiB 可能超过限制。先测；超了就分块，不降低上限。
2. **secret**：`providerService`、`mcpAuth`、OAuth 服务保持 internal；数据面只暴露操作，不暴露值。
3. **Workflow 未实现**：`workflowRuns` 是接口预留，不阻塞其余五项。
4. **`Faces` 的传播**：`Names`、`PartRegistry`、`SessionResolver` 都要拿到同一份原因表；独立 host / 测试用空表，检查只在装配和 leaf 加载处生效。
5. **legacy 标记**：84 个名字里 16 个是“数据内容、无消费者”，先标 `legacy-no-consumer`；不是永久 internal，出现消费者时改。
6. **catalog 的 `is_shared` 与 dispatcher**：`managementMethods` 证明两者可能不一致；#1289 要求“有 dispatcher 就必须 shared”，否则检查无法用 `is_shared` 判断数据面。

## 7. 交付顺序

1. **#1289**：`Faces`、`AgentBuilder::internal`、catalog `face`、`xtask check`、leaf 拒绝、模板/清单。此 PR 只加检查和标记，不改任何服务行为。
2. **#1300**：按 §3 的分组在 catalog 里补齐 reason；`xtask graph` 的 diff 就是盘点结果。此 PR 不改服务。
3. **#1292**：按 4.1 → 4.2 → 4.5 → 4.3 → 4.6 的顺序，一项一个 PR；4.4 随 Workflow 落地。
4. 每项都带现有 flow 测试；不新增适配层，不写第二份逻辑。
