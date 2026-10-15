# PluginManager

状态：草案。总体设计：[overview §4 插件目录](../overview.md)，[Settings、Mcp、SkillsCatalog 变更](../overview.md)，[约定](../overview.md)，[多节点](../overview.md)，[ADR 002](../../adr/002-plugin-scope.md)。阶段：P3（依赖 [#3](https://github.com/arcships/dim/issues/3)；非 Rust Capability 插件依赖 [#4](https://github.com/arcships/dim/issues/4)；对端节点依赖 [#5](https://github.com/arcships/dim/issues/5)）。语言：Rust。

> **重新设计时的改动。** (1) [#36](https://github.com/arcships/dim/issues/36)：依赖方向反过来。PluginManager 依赖内容所有者（Mcp、SkillsCatalog 等），通过它们的类型化操作把插件包内容交给它们；所有者不依赖 `pluginContents`，PluginManager 失败或重载时，它们照样用用户和项目来源工作。(2) [#41](https://github.com/arcships/dim/issues/41)：PluginManager 在写入或重新启用插件包的行之前校验它；代码行不在 `apply` 中检查完整性（那时 loader 已经执行过入口）。启用的插件包是可信的（ADR 003 D4）；安装后对文件的篡改不在防护范围内。

> **P3：按 rutis 现有能力重新设计。** rutis 是外部依赖，本文向 rutis 提出的需求已撤回（[原则 §4 第 8 条](../../../architecture/rust-agent/principles.md)）。

PluginManager 是一个进程插件（[ADR 002](../../adr/002-plugin-scope.md)：安装按宿主进行）。它从 Git marketplace、本地路径或 Git URL 将插件包安装到目标节点上；通过类型化的 Settings 操作将插件意图（安装、启用、节点绑定、包路径）记录到 loader 行中；并将已启用包的类型化内容发布为命名进程服务 `pluginContents`。内容所有者（SkillsCatalog、Mcp、Hooks、Workflow，任意语言）按名称依赖 `pluginContents`，定义各自的内容类型 key，并读取它们能理解的内容；PluginManager 对它们一无所知。包启用后其代码即运行：没有宿主本地批准，因为安装和启用是经管理方法完成的宿主配置，不属于 Approval 与 Permission 的范围，后两者只管 agent 决策引起的动作（[overview 约定](../overview.md)）。包从不自动升级。PluginManager 只处理结构性操作（安装、启用、禁用、更新、移除），并且是任何插件行上结构性字段的唯一写入方，包括 [RemoteGateway](RemoteGateway.md) 等内置插件的 `enabled` 字段；每个插件声明自己业务配置的 schema，业务配置经 Settings 类型化操作写入它的各个具名 Settings 来源，从不经过 PluginManager。

## 1. 结构

```mermaid
flowchart LR
  subgraph CN["App Server 进程，控制节点"]
    PM["PluginManagerPlugin<br/>plugin.* 方法、包存储、<br/>pluginContents"]:::new
    SET["Settings<br/>loader 行上的类型化操作"]
    LOADER["rutis Loader"]
    RPC["Rpc 方法表<br/>收集 managementMethods"]
    CONS["内容所有者<br/>SkillsCatalog、Mcp、Hooks、Workflow"]
  end
  subgraph NODE["每个已配置节点，本地或对端"]
    SVC["该节点的<br/>FileStorage、HostPlatform"]
    CODE["包代码行<br/>启用时加载"]:::new
  end
  PM --> SET
  PM --> LOADER
  PM -->|"按节点逐个操作"| SVC
  RPC -->|"plugin.* managementMethods"| PM
  CONS -->|"按自有类型 key 取来源、changed"| PM
  CODE -->|"在 apply 中校验完整性"| SVC
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
```

所有依赖都是同一作用域的进程服务（[ADR 002 D3](../../adr/002-plugin-scope.md)）。内容所有者依赖 `pluginContents`，按 D3 予以接受：安装、启用或移除包会以数据更新（`changed`）的方式改变已发布的来源，从不触发重载；而 PluginManager 重载（升级、崩溃）会重载内容所有者。Rpc 按调用查找 PluginManager 的 `managementMethods` 服务（key `plugin`），不随它重载（[方法 §3.2](../contracts/session-boundary.md#32-管理方法)）。PluginManager 重载是少见的结构性事件，它向内容所有者的传播被记录在案，而非被阻止。包代码行是普通的 loader 行：它们提供其代码在 `provides` 中声明的命名服务（例如 `toolContribution`、`sessionMethods`、`modalityAdapter`），消费方按服务名和作用域收集它们；当某一行卸载时，其依赖方按同一规则重载。离线对端节点上的代码行不会卸载：其导入的服务保持注册并以 `node_unavailable` 使调用失败，其依赖方既不重载也不变为 Pending（对 [#5](https://github.com/arcships/dim/issues/5) 的要求，见 [§9](#9-向机制所有者提出的需求)）。

| 部分 | 作用域 | 依赖 | 提供 |
| --- | --- | --- | --- |
| `PluginManagerPlugin` | 进程，控制节点 | Settings（loader 可编辑层上的类型化操作）、控制节点上的 `pluginFiles`（扩展）、rutis Loader、每个已配置节点的 `FileStorage` 与 `HostPlatform` | `pluginContents` 进程服务；`managementMethods` key `plugin`（`plugin.*`），由 Rpc 收集 |
| 包代码行 | 进程，位于包所在节点 | 该节点的 `FileStorage`、代码所声明的依赖 | 在其 `provides` 中声明的命名服务；在使用时读取自己的业务配置（自己的各个具名 Settings 来源与 schema） |
| 内容所有者 | 进程 | `pluginContents` | —（各自定义自己的类型 key） |

此处定义的契约：包清单 `dim-plugin.json` schema 2、marketplace 索引 `dim-marketplace.json`、`pluginContents` 服务及其 `ContentSource` 形态（[语言中立](../../adr/002-plugin-scope.md)，A.3；Rust 绑定见附录 G）、包目录布局、代码行完整性检查、`plugin.*` 方法。依赖的契约：loader 可编辑层上的类型化 Settings 操作（#3），用于插件意图和结构性字段；控制节点上的 `pluginFiles`（扩展）；rutis-loader `Loader`（`entries`、`reload`、`restart`、`LoaderChanged`）；包所在节点的 `FileStorage` 与 `HostPlatform`（对端节点：经 rutis 链路导入）；[方法](../contracts/session-boundary.md)契约。这些被依赖的服务是现有的类型化 Rust 服务，PluginManager 作为 Rust 插件直接使用它们；将它们按名称暴露给非 Rust 代码行是 [#4](https://github.com/arcships/dim/issues/4) 的工作。PluginManager 从不调用内容所有者，也从不在代码中写出任何内容类型 key。

## 2. 数据与状态

```mermaid
flowchart TB
  subgraph LEL["Loader 可编辑层，经 Settings"]
    MK["Marketplace 注册"]
    PR["按节点和 id 的包意图<br/>版本、来源、commit、指纹、<br/>内容、启用；代码行"]
  end
  subgraph PS["每个节点上的包根目录，经其 FileStorage"]
    PKG["包目录<br/>按指纹内容寻址"]
    STG["暂存区"]
  end
  CACHE["Marketplace 克隆与索引<br/>控制节点 pluginFiles（扩展）"]
  MEM["pluginContents 来源与 revision<br/>内存"]:::new
  PR -->|"apply 及每次变更：校验"| PKG
  PKG -->|"发布"| MEM
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
```

loader 行中的包意图是持久状态；`plugin.list` 由 loader 条目投影而来，从不来自单独的存储。已发布的来源在 `apply` 时从意图重建（校验每个已启用包），并在每次操作后原地更新，同时递增 revision。每插件业务配置既不在行中，也不在 PluginManager 中：每个插件声明其 schema，配置经 Settings 类型化操作写入它的各个具名 Settings 来源，插件在使用时读取，因此配置从不重载任何东西（[ADR 002 D4](../../adr/002-plugin-scope.md)、A16）。包字节存放在其所在节点的包根目录下，marketplace 缓存在控制节点的 `pluginFiles`（扩展）中。没有任何内容是 Session 事实。

## 3. 流程

### 3.1 安装与发布

```mermaid
sequenceDiagram
  participant C as Client
  participant PM as PluginManager
  participant PS as Package store on node
  participant S as Settings
  participant L as Loader
  participant O as Content owner
  C->>PM: plugin.install 来源、节点
  PM->>PM: 锁定节点和 id
  PM->>PS: 暂存：拉取并校验摘要
  PM->>PS: 提交：原子 rename
  PM->>S: plugins.install，带期望 revision
  S->>L: 包意图与代码行
  PM->>PM: 发布来源，递增 revision
  PM-->>C: id、指纹
  PM-->>O: changed
  O->>PM: 取其自有类型 key 的来源
```

纯内容包一经发布即生效；所有者在 `changed` 时自行对齐，从不接收推送。已启用包的代码行立即加载；只有在其运行时不可用时才处于 Pending。

### 3.2 启用、禁用与代码生命周期

```mermaid
sequenceDiagram
  participant C as Client
  participant PM as PluginManager
  participant S as Settings
  participant L as Loader
  participant K as Code row
  C->>PM: plugin.set_enabled id、true
  PM->>S: plugins.set_enabled，带期望 revision
  S->>L: 启用代码行
  L->>K: apply 校验包的完整性
  C->>PM: plugin.set_enabled id、false
  PM->>S: plugins.set_enabled
  S->>L: 禁用代码行
  L->>K: dispose
  PM-->>C: dispose 完成后返回
```

代码行在自身的 `apply` 中检查完整性；不匹配时以 `integrity_failed` 失败并停止服务。不询问批准：包之所以运行，是因为本地管理员启用了它。禁用代码会卸载该行；其依赖方按 D3 重载，使用了它所提供内容的在途轮次可能失败。该代价被记录在案，而非被阻止。

### 3.3 显式更新，从不自动

```mermaid
sequenceDiagram
  participant C as Client
  participant PM as PluginManager
  participant PS as Package store
  participant S as Settings
  C->>PM: plugin.marketplace.update
  PM->>PS: 仅重新拉取索引
  PM-->>C: commit、updateAvailable 标记
  C->>PM: plugin.update id、expectedRevision
  PM->>PS: 暂存并提交新的指纹目录
  PM->>S: plugins.update 版本、指纹
  Note over PM,S: 代码行在校验后基于新的指纹目录重启
```

marketplace 刷新只标记 `updateAvailable`；更改已安装的包始终需要显式的 `plugin.update`（决策 E22）。

## 4. 时序与冲突

```mermaid
sequenceDiagram
  participant PM as PluginManager
  participant PS as Package store
  participant S as Settings
  participant C as Client
  PM->>PS: 提交
  alt 在 Settings 写入前中断
    PM-->>C: operation_interrupted，无意图，目录稍后回收
  else 在 Settings 写入后中断
    PM-->>C: operation_interrupted，意图已存在，重试得到 plugin_exists
  end
```

安装的顺序为暂存、提交、Settings 写入；Settings revision 是提交点，因此中断从不会留下半注册的包。

| 场景 | 结果 | 负责方 |
| --- | --- | --- |
| 代码行在其提供的内容正被使用时被禁用 | 该行卸载；依赖方重载（D3）；受影响的在途轮次可能失败 | Loader、代码行 |
| 代码运行时包被更新 | 代码行基于新的指纹目录重启；依赖方重载（D3） | Loader、代码行 |
| 安装后节点离线 | 该节点上的操作 `node_unavailable`；其来源被撤回；其代码行的服务以 `node_unavailable` 使调用失败；不发生重载，也没有任何东西变为 Pending；其他节点不受影响 | PluginManager |
| 插件运行时其业务配置变更（Settings 类型化操作） | 业务数据更新与通知；不重载；下次使用读取新值 | 插件、Settings |
| 同一 id 的两次安装 | 第二次等待锁，然后 `plugin_exists` | PluginManager |
| 分支使用某包的 Skill 时该包被禁用 | 分支保留其已采纳的提示词，直至重新采纳 | Skills 所有者 |
| PluginManager 重载 | 内容所有者随之重载（D3）；重新激活前 `plugin.*` 返回 `unknown_method`，Rpc 不重载；在途的 install/remove `operation_interrupted` | PluginManager |

完整表格：[附录 B](#附录-b-完整冲突表)。

## 5. 上下文映射

无直接的模型可见效果。PluginManager 不写输入，不提供工具，也不注册提示词来源。包内容只通过其所有者、以该所有者的形式到达模型：Skills 通过 Skill 提示词来源（按分支采纳，压缩后作为配置重放，子分支从已采纳前缀继承），MCP 与代码行工具通过工具声明和 `ToolResult.content`，Hooks 输出通过 Session 输入。非交互 Session 的差异仅由所有者定义。模型从不看到包 id、指纹、marketplace 数据、配置或诊断。

## 6. 接口（摘要）

| 接口 | 类型 | 用途 |
| --- | --- | --- |
| `plugin.marketplace.list/add/remove/update/search/view` | 管理方法 | Marketplace；`update` 仅刷新索引 |
| `plugin.install/update/list/set_enabled/reload/remove` | 管理方法 | 包生命周期；`set_enabled` 也用于开关内置插件行 |
| `pluginContents`（`sources`、`changed`） | 进程服务，无 key | 通用内容发布 |
| 代码行完整性检查 | Loader 行规则 | 代码行在启用且包校验通过时加载 |
| `dim-plugin.json` v2、`dim-marketplace.json` v1 | 文件格式 | 包与 marketplace 描述 |
| 类型化 Settings 操作 `plugins.*`、`marketplaces.*` | 依赖（#3、ADR 002 D4） | 持久化意图（仅结构性） |
| CLI | `/plugins`、`/plugin`、`/reload-plugins`、`dimcode plugin …` | 用户入口 |

详见：[附录 A](#附录-a-接口细节)。

## 7. 安全

安装、启用和移除是管理方法，只由本进程插件提供，从不提供给已配对的远程设备（[方法](../contracts/session-boundary.md)）；RemoteGateway 从不提供它们。包代码一旦加载即为受信任的原生进程代码（AGENTS 规则 11）。它运行是因为本地管理员安装并启用了它，不是 agent 的决策，因此不归 Approval 与 Permission 管，也没有宿主本地批准：这项配置的安全由管理员负责。Git 拉取不运行任何仓库 hooks 或子模块；安装时从不执行包文件；每次加载都重新检查完整性。内容所有者在其解释执行的可执行内容（Hooks、MCP、workflows）启用时运行它，没有批准步骤。插件的业务配置（在它的各个具名 Settings 来源中）只保存凭据引用，由插件通过 Settings 凭据解析器解析；PluginManager 从不读取它，解析后的秘密从不进入行、`plugin.list`、诊断或内容来源。代码贡献管理方法的包只有在本地管理员启用后才扩展宿主管理面。

## 8. 规则检查

规则：[principles §1 与 §4](../../../architecture/rust-agent/principles.md)，[ADR 002](../../adr/002-plugin-scope.md) D1–D5。

| 规则 | 本文如何符合 |
| --- | --- |
| §4.1 改别的模块拥有的状态时写指令事件 | loader 行是没有 Timeline 的进程状态；PluginManager 只通过所有者的类型化写入操作（基于 #3 的 Settings `plugins.*`）改动它们，从不写入其他插件的业务配置。 |
| §4.2 交接后不能撤回 | 不适用：PluginManager 不向其他模块交接 Timeline 对象；已发布来源的变化只在所有者重新采纳时才影响分支。 |
| §4.3 只用 before 回调在写入前干预 | 不适用：PluginManager 不注册 before 回调，也不在 Session 或 RPC 入口加检查；代码行完整性检查在该行自己的 `apply` 中进行。 |
| §4.4 核心机制不带业务含义 | `pluginContents` 发布不透明的类型化来源；Loader 与 Rpc 不为任何插件做特殊处理。 |
| §4.5 决定前读完整个批次 | 不适用：PluginManager 不读任何 Timeline 事件。 |
| §4.6 模型上下文只含交接后被读取的事件 | 包内容只通过其所有者已采纳的提示词、工具或输入规则到达模型。 |
| §4.7 插件之间只通过公开事件和服务交互 | PluginManager 从不调用内容所有者；所有者读取 `pluginContents`，各插件的业务配置只经 Settings 访问。 |
| §4.8 向机制所有者提通用需求 | 类型化 loader 操作（#3）、为非 Rust 代码按名字暴露服务（#4）与节点离线行为（#5）在 §9 中提出。 |
| §4.9 只有后续决定要用的事实才进 Timeline | PluginManager 不写 Timeline 事件；意图留在 loader 行，包留在节点存储，客户端读取 `plugin.list`。 |
| §1 Capability 与 App Server 边界 | PluginManager 是位于自己 crate 的进程插件，不是 Capability；App Server 只收集它的 `managementMethods`。 |
| ADR 002 D1 作用域跟随状态所有者 | 安装按宿主进行，因此 PluginManager 是进程作用域。 |
| ADR 002 D2 不设接线模块 | 内容所有者与代码行的消费方按名字收集服务；没有接线模块。 |
| ADR 002 D3 依赖跟随作用域 | 所有依赖都是进程服务；PluginManager 与代码行的重载传播是已记录的少见结构性事件。 |
| ADR 002 D4 结构性配置与业务配置分离 | PluginManager 只向 loader 行写入结构性意图（安装、启用、禁用、更新、移除），并且是 RemoteGateway 的 `enabled` 等结构性字段的唯一写入方；业务配置属于各插件的各个具名 Settings 来源。 |
| ADR 002 D5 合同语言中立 | `pluginContents`、清单与各方法以中立形式给出；附录 G 只是绑定。 |

## 9. 向机制所有者提出的需求

### 9.1 loader 可编辑层上的类型化操作（[#3](https://github.com/arcships/dim/issues/3)）

已撤回：rutis 是外部依赖，不向它提需求。PluginManager 使用 rutis-loader 现有的操作，作为唯一写入方自己检查版本，并在写入行之前校验插件包完整性。

| 项 | 内容 |
| --- | --- |
| 被哪条规则挡住 | ADR 002 D4 把结构性插件意图放在 loader 行中，principles §4.1 与 §4.7 禁止直接修改别的模块的状态；rutis-loader 0.10 没有增加、更新或删除行的类型化写入操作。 |
| 考虑过的现有机制 | `Loader` 的 `entries`、`reload`、`restart` 只能读取或重启行；`VolatileUpdate` 只改已有行的 `x-volatile` 字段，不能增删行；Settings 业务来源存放业务配置，不存放行；PluginManager 自有的存储会成为行的第二个事实来源。 |
| 需要的通用能力 | 可编辑层上的类型化操作：增加、更新、删除一行，设置其 `disabled` 标志，以及设置已有行的某个字段（`x-volatile` 字段以 `VolatileUpdate` 送达，不重载），每个操作带预期 revision，原子执行，并发出变更通知；不了解包或插件。 |
| 对序号、批次、中断、重载的影响 | 不涉及 Timeline 序号或批次；被中断的操作要么已提交其 revision，要么什么都没写；增删一行只装载或卸载该行，其依赖方按 D3 重载。 |
| 由谁实现，对应 issue | rutis-loader，[#3](https://github.com/arcships/dim/issues/3)。包装它的 Settings 类型化操作（A.6）是 Settings 补充，列在 [overview 附录 B.2](../overview.md)。 |

### 9.2 其他需求（在别处写明）

| 需求 | 所有者 | 五项内容 |
| --- | --- | --- |
| 从对端节点导入的服务在链路断开期间保持登记，调用以 `node_unavailable` 直接失败 | rutis，[#5](https://github.com/arcships/dim/issues/5) | 见 [ExternalAgent](ExternalAgent.md) §9；PluginManager 需要它，使离线节点从不重载 PluginManager，也不使代码行的依赖方变为 Pending |
| 运行时服务按名字暴露给非 Rust 插件 | 运行时，[#4](https://github.com/arcships/dim/issues/4) | 见 [session access](../contracts/session-access.md) §9；Node 代码行需要它来使用 `FileStorage` |

## 附录 A. 接口细节

### A.1 包清单 `dim-plugin.json`（schema 2）

版本 1 是现有的托管提供方清单（[App Server 契约](../../../../crates/dim-app-server/README.md#optional-managed-provider)）；它保留给 `--plugin-dir` 使用，在此不可安装（A3）。

```json
{
  "schemaVersion": 2,
  "id": "acme-review",
  "version": "1.2.0",
  "name": "Acme review kit",
  "description": "Review Skills and hooks",
  "files": {"skills/review/SKILL.md": "<sha256>", "dist/index.js": "<sha256>"},
  "contents": [
    {"type": "skills", "path": "skills"},
    {"type": "hooks", "path": "hooks/hooks.json"}
  ],
  "code": [
    {"runtime": "node", "entry": "dist/index.js", "hostApiVersion": 2, "rutisVersion": "0.10.0"}
  ]
}
```

| 字段 | 规则（在安装边界校验一次） |
| --- | --- |
| `id` | `[a-z][a-z0-9-]{0,63}`；每节点唯一（A4） |
| `version` | SemVer；仅用于显示和比较 `updateAvailable` |
| `files` | 除 `dim-plugin.json` 和 `.git/` 外的所有文件；相对 POSIX 路径，不含 `..`，无符号链接；未列出的文件或摘要不匹配即拒绝 |
| `contents` | 不透明的 `type` `[a-z][a-z0-9-]*`（key 由其消费方定义：`skills`、`hooks`、`mcp`、`workflows`，R14），包内的 `path`，可选 `options` ≤ 64 KiB；只校验这一形状 |
| `code` | 可选；`runtime: "node"` 将 `entry` 作为 rutis 运行时行加载；检查 `hostApiVersion`/`rutisVersion`；其他运行时在 #4 之前被拒绝（A1） |
| 指纹 | 所存清单字节的 SHA-256；清单列出了每个文件的摘要，因此它绑定全部包字节。它在 `plugin.list` 和 `pluginContents` 中标识确切的包内容，并用于给包目录寻址 |

### A.2 Marketplace 索引 `dim-marketplace.json`（A5）

位于仓库根目录：`{schemaVersion: 1, name, plugins: [{id, version, description, keywords?, source: {path} | {git, ref?}, fingerprint?}]}`。`source.path` 位于 marketplace 仓库内；`fingerprint` 若存在，必须等于所拉取包的指纹。

### A.3 `pluginContents`

服务 `pluginContents`，作用域为进程，无 key，由 PluginManagerPlugin 提供；内容所有者按名称依赖它。错误是数据 `{ code, message }`。

| 方法 | 类型 | 请求 | 回复 |
| --- | --- | --- | --- |
| `sources` | 同步 | `{ type }` | `{ revision, sources: ContentSource[] }`：可用节点上已启用且通过完整性校验的包 |
| `changed` | 异步 | `{ after_revision }` | 一旦已发布的 revision 超过 `after_revision` 即返回 `{ revision }`；消费方通过丢弃该调用来取消 |

`ContentSource`：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `package` | string | 包 id |
| `version` | string | 包版本 |
| `fingerprint` | string | 标识确切的包字节 |
| `node` | string 或 null | null = 控制节点 |
| `root` | string | `node` 上的包目录 |
| `path` | string | 声明的路径，相对于 `root` |
| `options` | object | 来自清单 |

消费方拥有自己的类型 key 与格式，并通过对 `node` 有效的服务读取文件。节点规则（A6）：节点 N 上的来源是节点 N 的内容。针对 Session 执行内容的消费方（例如 Hooks）只使用节点等于该 Session 资源节点（`SessionConfig.resource_node`）的来源；只在控制节点上读取内容的消费方忽略它无法触达的来源，并在自己的目录中报告它们。

### A.4 包存储

PluginManager 内部组件，不是服务：`stage`（拉取、校验）、`commit`（原子 rename）、`verify`（重新哈希）、`remove`、`fetch_marketplace`，每项都针对目标节点的 `FileStorage` 与 `HostPlatform` 运行；来源是本地路径，或带可选引用的 Git URL。未配置或其服务不可用的节点会使操作以 `node_unavailable` 失败。

包存放在其所在节点上 PluginManager 的包根目录下，通过该节点的 `FileStorage` 写入（A13）。Git 通过该节点的 `HostPlatform` 运行，禁用 hooks 和子模块，仅限 `https://` 和 `ssh://`（A7）。本地路径被复制，从不被引用（A8）。目录按指纹内容寻址，因此更新从不改写运行中行正在使用的文件。

### A.5 方法（`managementMethods`，key `plugin`）

| 方法 | 参数 | 结果 | 中断 |
| --- | --- | --- | --- |
| `plugin.marketplace.list` | `{}` | `[{name, url, ref, commit, fetchedAt, error?}]` | Drain |
| `plugin.marketplace.add` | `{name, url, ref?}` | `{commit}` | Report |
| `plugin.marketplace.remove` | `{name}` | `{}` | Report |
| `plugin.marketplace.update` | `{name?}` | `[{name, commit, error?}]`；仅刷新索引 | Report |
| `plugin.marketplace.search` | `{query, marketplace?}` | `[{marketplace, id, version, description, installed: [{node, version}]}]` | Drain |
| `plugin.marketplace.view` | `{marketplace, id}` | 条目加清单摘要（`contents` 类型、是否有 `code`） | Drain |
| `plugin.install` | `{source: {marketplace, id} \| {path} \| {git, ref?}, node?, enabled?}` | `{id, version, node, fingerprint}` | Report |
| `plugin.update` | `{id, node?, version?, expectedRevision}` | `{version, fingerprint}` | Report |
| `plugin.list` | `{node?}` | `[{id, version, node, builtin, enabled, status, contents, source, updateAvailable, error?}]`；内置插件行以 `builtin: true` 出现 | Drain |
| `plugin.set_enabled` | `{id, node?, enabled, expectedRevision}`；`id` 可以指内置插件行（A17） | `{}` | Report |
| `plugin.reload` | `{id?, node?}` | `[{id, node, status, error?}]` | Report |
| `plugin.remove` | `{id, node?}` | `{cleanupError?}` | Report |

`status`：`active`、`pending`（等待运行时）、`unavailable`（节点离线；其服务使调用失败）、`failed`（原因）、`disabled`。`install`、`update` 和 `marketplace.update` 通过该调用的进度接收器报告阶段（`fetch`、`verify`、`commit`）（[方法](../contracts/session-boundary.md)，A14）。相对 overview 目录条目新增：`update`、`marketplace.search/view`（A9）。

### A.6 使用的类型化 Settings 操作（#3，拟议的 Settings 补充）

| 操作 | 效果 |
| --- | --- |
| `plugins.install(entry, expected_revision)` | 记录包意图，并在可编辑层创建代码行 |
| `plugins.update(id, node, version, fingerprint, expected_revision)` | 更新包意图；代码行随之变化 |
| `plugins.set_enabled(id, node, enabled, expected_revision)` | 设置包的启用标记，并对其代码行设置 `set_disabled`；对内置插件行，设置其结构性 `enabled` 字段（为 `x-volatile` 时以 `VolatileUpdate` 送达） |
| `plugins.remove(id, node, expected_revision)` | 移除包意图及其所有代码行 |
| `marketplaces.save/remove` | Marketplace 注册 |

`reload`、`restart` 与状态查询直接调用 `Loader`。

### A.7 Capability 机制

| 步骤 | 路径 |
| --- | --- |
| Timeline | 无写入，无 Dispatcher 订阅，无 `StateCell`；复查依据是 Settings revision |
| 安装 | 锁定 `(node, id)`（A10），`stage`，与意图比较，`commit`，`plugins.install`，发布来源，回复；Settings 写入失败会留下一个未被引用的目录，稍后回收（A11） |
| 发布 | `apply` 时校验每个已启用包并构建来源；每次操作后更新来源并递增 revision；校验失败的包被撤回并报告为 `failed` |
| 代码行 | `apply` 校验完整性，否则以 `integrity_failed` 失败；`set_enabled`/`update` 通过 `Loader` 重启它们；每个代码行在使用时读取自己的业务配置 |
| PluginManager 重载 | 内容所有者随之重载（D3）；重新激活前 `plugin.*` 返回 `unknown_method`，Rpc 不重载；在途的 install/remove 返回 `operation_interrupted`；代码行不受影响 |
| 进程重启 | Loader 恢复各行；包重新校验；暂存区删除；孤立目录在 loader 稳定后回收 |

## 附录 B. 完整冲突表

| 场景 | 序列顺序 | 结果 | 决策负责方 |
| --- | --- | --- | --- |
| 轮次或分支中断、分支关闭、并发输入、`input/withdraw`/`input/deleted`、未结算批次、压缩、子分支切分 | — | 对 PluginManager 无影响；适用所有者的采纳规则 | 内容所有者 |
| Session 使用某包的 Skill 时该包被禁用 | revision 递增 < 下次采纳 | 分支保留其已采纳的提示词，直至重新采纳 | Skills / PromptSourcesBranch |
| 代码行在使用中被禁用 | — | 该行卸载；其依赖方重载（D3）；受影响的在途轮次可能失败 | Loader |
| 同一节点上同一 id 的两次安装 | 锁，然后 Settings revision | 第二次 `plugin_exists`（跨进程时为 `plugin_settings_conflict`） | PluginManager、Settings |
| 代码运行时包被更新 | 新指纹 | 代码行基于新目录重启；依赖方重载（D3） | Loader、代码行 |
| 代码运行时禁用 | 禁用已持久化 < 重启 | 代码行在结算其工作后 dispose；dispose 后回复 | Loader、代码行 |
| 安装在提交后、Settings 写入前中断 | — | `operation_interrupted`；无意图；目录被回收 | PluginManager |
| 安装在 Settings 写入后中断 | — | `operation_interrupted`；意图已存在；重试 `plugin_exists` | 客户端读取状态 |
| 重复或迟到的 `plugin.install` | — | `plugin_exists` | PluginManager |
| Marketplace 更新发现更新的版本 | — | 仅 `updateAvailable`；不做任何改变 | PluginManager |
| 安装前目标节点不可用 | — | `node_unavailable`；不写入任何内容 | PluginManager |
| 安装后节点离线 | — | 其来源被撤回；其代码行导入的服务保持注册并以 `node_unavailable` 使调用失败；没有依赖方重载或变为 Pending（对 #5 的要求）；其他节点不受影响 | PluginManager、rutis 链路 |
| 包所在节点与 Session 的资源节点不同 | — | 执行型消费方对该 Session 忽略此来源 | 内容所有者 |
| 磁盘上的文件被修改 | 下次校验 | `integrity_failed`；来源被撤回；代码行失败 | PluginManager、代码行 |
| 操作中途 PluginManager 重载或重启 | — | 按 A.7；从不重新执行拉取 | PluginManager |
| Session 卸载/恢复 | — | 无影响 | — |
| 客户端脱离 | — | 操作继续；回复与进度被丢弃 | Rpc |

## 附录 C. 错误与恢复

| 代码（`appError`） | 含义 |
| --- | --- |
| `manifest_invalid` | 清单或索引缺失或格式错误 |
| `integrity_failed` | 摘要不匹配、未列出的文件、路径逃逸或符号链接 |
| `incompatible_host` | `hostApiVersion`/`rutisVersion` 或不支持的 `runtime` |
| `source_unavailable` | Git 拉取或路径读取失败（stderr 仅出现在诊断中） |
| `node_unavailable` | 目标节点未配置或其服务不可用 |
| `plugin_exists`、`plugin_not_found`、`marketplace_not_found` | 身份错误 |
| `plugin_settings_conflict` | 过期的 `expectedRevision` |
| `runtime_unavailable` | Node 运行时未配置；代码行 Pending |
| `operation_interrupted` | 方法契约的 `Report` 规则；重试前先读取 `plugin.list` |

移除先 dispose 代码行，再删除目录；删除失败时返回 `cleanupError`，并由垃圾回收重试。

## 附录 D. 验收

通过 `plugin.*` 方法进行流程测试，使用一个本地测试 marketplace 和一个自定义类型 key 的测试消费方：

1. 添加、更新、搜索、查看 marketplace；无效索引 `manifest_invalid`；更新标记 `updateAvailable` 而不改变已安装的包。
2. 从 marketplace、路径和 Git URL 安装；被篡改、含未列出文件和指纹不匹配的包被拒绝，且不写入任何内容。
3. 纯内容包通过 `changed()` 向测试消费方发布来源而不重载它；禁用会撤回来源；移除先删除意图再删除文件；PluginManager 重载会重载测试消费方，之后它读取到相同的来源。
4. 含 Node 代码的包一启用就加载其代码行，没有批准步骤；`plugin.update` 基于新目录重启它；禁用会 dispose 它；对 RemoteGateway 内置行调用 `plugin.set_enabled` 会以 `VolatileUpdate` 送达，不重载。
5. 禁用一个其代码行提供测试工具的包会重载其依赖方；测试将失败的在途轮次记录为已记录在案的结果。
6. 并发安装；过期的 `expectedRevision`；在提交后和 Settings 写入后中断。
7. 对端测试节点：安装以其为目标；使其离线时只有它的操作及其代码行的服务以 `node_unavailable` 失败，其来源被撤回，且不发生任何重载；恢复后复原。
8. 重启：各行恢复，完整性已校验，暂存区移除，孤立目录被回收。

## 附录 E. 假设

| # | 假设 | 依据 | 若被否决的影响 |
| --- | --- | --- | --- |
| A1 | 包代码是提供和消费命名服务的 Node 运行时行（rutis-bridge）；无动态 Rust 代码；按名称访问运行时的类型化服务需等待 #4 | Overview §4、附录 C；ADR 002 D5 | 更多 `code.runtime` 取值 |
| A2 | 包启用后其代码即运行；没有宿主本地批准 | 决策 R6（第二轮）；overview 约定 | 代码加载前增加批准步骤 |
| A3 | 清单 schema 2 扩展现有格式；v1 仅用于 `--plugin-dir` | App Server README；判断 | 兼容映射 |
| A4 | 每个 `(node, id)` 只安装一个版本 | 判断 | 并存多版本 |
| A5 | Marketplace 索引的格式与名称 | 判断；功能清单“Git plugin marketplaces” | 格式被替换 |
| A6 | 来源是节点内容；执行型消费方匹配 Session 的资源节点 | Overview 多节点；判断 | 消费方经导入的服务读取对端内容 |
| A7 | Git 使用节点的 `git`，禁用 hooks/子模块，仅限 `https`/`ssh` | 判断 | 库形式的拉取器或更多协议 |
| A8 | 本地路径被复制而非链接 | 判断（完整性） | 开发用链接模式 |
| A9 | 新增方法 `update`、`marketplace.search/view` | 功能清单；overview §4 | 并入其他方法 |
| A10 | 操作按 `(node, id)` 串行化 | 判断 | 仅依赖 Settings revision |
| A11 | 未被引用的目录在 loader 稳定后被垃圾回收 | 判断 | 显式清理命令 |
| A12 | 内容所有者依赖 `pluginContents`；PluginManager 重载会重载它们 | ADR 002 D3 | 单独托管的发布服务 |
| A13 | 包目录通过节点的 `FileStorage` 写入 PluginManager 包根目录下（该处可用原子 rename） | 判断 | 需要每节点存储服务 |
| A14 | 安装与更新报告进度阶段 | 方法契约进度接收器；判断 | 仅返回结果 |
| A15 | 无自动升级，包括纯内容包 | 决策 E22 | 可选启用的自动升级设置 |
| A16 | 每个插件声明其业务配置的 schema，业务配置经 Settings 类型化操作写入它的各个具名 Settings 来源，在使用时读取；PluginManager 没有 configure 方法 | ADR 002 D4 | PluginManager 将写入其他插件的业务配置 |
| A17 | `plugin.set_enabled` 也设置内置插件行（例如 RemoteGateway）的结构性 `enabled` 字段；任何插件都不写自己的结构性字段 | 决策 R3（第二轮）；ADR 002 D4 | 每个插件自己写自己的结构性字段 |

## 附录 F. 待对齐问题

| 问题 | 描述 |
| --- | --- |
| #3 类型化操作 | A.6 中的 Settings 操作在此提出；其最终形态属于 Settings 补充与 #3（仅结构性意图；业务配置归各插件，ADR 002 D4） |
| Overview SkillsCatalog 变更 | “安装与移除由 PluginManager 发起”理解为：PluginManager 发布和撤回来源，SkillsCatalog 读取它们；PluginManager 不调用 SkillsCatalog |
| 对端节点内容 | 控制节点上的消费方是否应通过导入的资源服务读取对端节点上的内容（#5） |

对 [#3](https://github.com/arcships/dim/issues/3)、[#4](https://github.com/arcships/dim/issues/4) 与 [#5](https://github.com/arcships/dim/issues/5) 的要求：见 [§9](#9-向机制所有者提出的需求)。

## 附录 G. Rust 绑定

Rust 内容所有者可以使用生成的绑定；A.3 中的中立形式是规范性的。

```rust
/// Binding for service `pluginContents` (process, no key).
impl PluginContents {
    pub fn sources(&self, type_key: &str) -> Vec<Arc<ContentSource>>;
    pub fn changed(&self) -> tokio::sync::watch::Receiver<u64>; // revision; maps `changed`
}
pub struct ContentSource {
    pub package: String,
    pub version: String,
    pub fingerprint: String,
    pub node: Option<String>,
    pub root: PathBuf,
    pub path: PathBuf,
    pub options: serde_json::Value,
}
```

PluginManager 将其注册在 `TypeKey::named("pluginContents")` 下；其 `plugin.*` 方法使用[方法](../contracts/session-boundary.md)契约的 `managementMethods` 绑定。
