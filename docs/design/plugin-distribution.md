# 插件发现、分发与作者入口

作用域：process。设计讨论与交付由 [#1410](https://github.com/arcships/dim/issues/1410) 跟踪。

本文提出从发现插件到显式安装的跨模块契约。新增接口和字段是提案，不能作为已发布命令使用；各所有者在实现前按第 8 节完善自己的契约。现有安装方式见 [PluginManager](plugins/PluginManager.md)，现有作者接口见 [TypeScript 插件编写指南](../typescript-plugin-authoring.md)。

## 1. 目标与归属

用户能搜索插件、核对来源和兼容性，再通过 CLI 或受支持的客户端安装。官方插件可以继续放在 Dim 主仓库的子目录里；可发现、可安装与是否随产品打包、是否启用分别决定。Workflow 是这一流程的验收包，基础 CLI 仍不交付它及其私有依赖。

| 内容 | 所有者 | 本设计提出的要求 |
| --- | --- | --- |
| 官方目录、社区发现、来源解析 | PluginManager | 两种发现来源共用一个有界目录读入口，解析为同一种安装来源 |
| 安装记录、选择、启用、更新、移除 | [PluginManager](plugins/PluginManager.md) | 扩展 Git 来源的包子目录及发现来源信息，不增加第二份安装记录 |
| 解释器选择、下载、校验、运行时包 | [RuntimeManager](plugins/RuntimeManager.md) | 增加本机优先策略，保留显式来源选择 |
| 加载、依赖等待、代际释放 | [rutis 使用契约](rutis-usage.md) | 直接使用既有机制，不增加 loader 或状态机 |
| 插件设置、业务数据、内容 | 插件及各[内容所有者](plugins/PluginManager.md#24-state-pluginmanager-does-not-keep) | 分发不读取或迁移业务数据 |
| 命令与界面 | [CLI](cli-commands.md)、各客户端 | 展示所有者回复，不维护客户端安装器或目录真相源 |

不增加市场托管服务、通用包仓库、插件依赖求解器、远程官方目录同步或新的装配服务。官方插件不需要迁出主仓库，也不要求改写为 Rust。Desktop 默认选择由 [#1356](https://github.com/arcships/dim/issues/1356) 决定。

## 2. 发现与身份

### 2.1 官方目录

官方目录是仓库维护的一份 JSON，在构建时嵌入程序。每项包含稳定目录 ID、插件名称、搜索摘要和 Git 安装来源；绑定示意见附录 A。目录在无网络时仍可查询，安装所需的网络不可用则单独报告。

官方身份只来自内置目录。社区仓库的名称、topic、作者自述或 manifest 都不能声明自己为官方。每个目录项的来源必须精确到仓库、版本引用和包目录；发布校验检查来源指向的包名与目录项一致。

发布版本的目录引用经过验证的兼容 tag 或 commit，不指向可漂移的默认分支。构建可使用同次 Dim 发布的 tag；发布校验在 tag 已解析到发布提交后验证插件来源，不能要求提交包含自己的 SHA。开发构建使用显式来源，不将未发布版本伪装成可安装条目。

目录变更随 Dim 发布。新二进制带来的目录变化只改变可用候选，不自动安装、启用或更新已有插件，也不覆盖用户选择的其他来源。

### 2.2 GitHub 社区发现

社区发现通过 GitHub 的 `dim-plugin` repository topic 查询公开仓库。topic 用于索引，与选择发布版本的 Git tag 不同。

首期自动发现读取仓库默认分支根目录的 `dim-plugin.json`；根目录没有有效 manifest 的仓库不作为可安装条目。只读取元数据，不运行入口、构建脚本、包管理器或 runtime。社区 monorepo 可通过显式 Git URL 和子目录安装；首期不递归扫描整仓库，也不引入第二种仓库索引文件。

仓库定位使用 GitHub 返回的明确 owner/repository。展示名称不作为全局身份；同名插件保留各自来源，不按星数替用户选择。默认分支用于发现，详情将用户选择的 ref 解析到具体 commit，并从该 commit 读取包信息。

搜索由 App Server 执行并限制分页与网络工作量；客户端不抓取仓库。可用短期缓存减少重复读取，缓存只保存可重新获取的目录数据，不构成安装或授权记录。回复区分来源正常、缓存结果和来源不可用，并给出读取时间。GitHub 限流或失败时，官方结果仍可返回，但必须同时标明社区结果不完整；不能用空列表表示网络失败。

### 2.3 目录项、来源与已安装实例

| 身份 | 用途 |
| --- | --- |
| 目录项 ID | 选择发现结果，如官方 `workflow` 或社区 `owner/repository`；所属来源同时参与标识 |
| Git 仓库与包子目录 | 标识一个可获取的包；用于合并来自两种目录的同源条目 |
| 解析后的 commit | 标识此次获取的仓库内容；与子目录一起确定实际包 |
| manifest 名称 | 沿用 PluginManager 的同名选择与替换语义 |
| 已安装 ID | 沿用 PluginManager 的本地实例身份，供更新和移除使用 |

官方短名只解析内置目录；社区安装使用明确的 owner/repository，或详情返回的来源。两种来源指向同一仓库和子目录时合并展示，并保留官方来源信息。仅 manifest 同名不能合并。一个官方仓库包含多个子目录插件时，每个目录项独立存在。

目录消失、topic 被移除或离线都不撤销已有安装。安装记录保存实际 Git 来源及目录来源信息，更新和移除不依赖搜索结果仍然存在。

## 3. 可独立安装的包

插件包继续使用 [PluginManager 的 manifest](plugins/PluginManager.md#21-the-plugin-directory-and-its-manifest)，不另建插件格式或 runtime 解析器。选中的目录就是包根目录；入口、内容资源和私有依赖必须能在该根目录独立解析。

作者发布前准备好入口及需要构建的资源。Git 安装不替作者执行任意构建或生命周期脚本。TypeScript 可使用宿主已支持的入口形式；代码和依赖不得依赖宿主源码 checkout、仓库父目录、未随包交付的 workspace 包或宿主的开发依赖。

生产依赖沿用[现有打包与信任契约](../typescript-plugin-authoring.md#packaging-and-trust)：可随包交付，或采用 PluginManager 明确支持的锁定依赖安装形式。Python runtime 可获取不代表任意 Python 插件依赖已能自动安装；没有受支持依赖契约的包必须自行准备可分发内容，安装器明确拒绝不能满足的声明。不得用全局 npm/pip 安装补洞。

发现详情需要说明包版本、支持的 Dim 版本、宿主插件协议/SDK 兼容范围，以及所需解释器约束。兼容声明归 manifest，目录不再复制一份。它们只判断宿主能否加载候选，不为一组插件求解或升级宿主依赖。缺少兼容声明的历史包可以保留现有直接路径安装语义，但不能在新目录里被标成“已确认兼容”。

这些新增字段须由 manifest 所有者定义版本化解析规则及旧客户端行为。不能把旧的严格读取器不认识的字段直接放进仍声明兼容旧 Dim 的包。预发布 Dim 版本必须按明确的兼容规则匹配，不能当作稳定版范围自动接纳。

## 4. Git 子目录获取与安装

### 4.1 来源定位

Git 来源扩展为仓库 URL、可选 ref 和包子目录；未指定子目录仍表示仓库根目录。网络协议和凭据处理沿用 [PluginManager 获取边界](plugins/PluginManager.md#7-security)。

子目录是仓库内相对路径。获取边界拒绝绝对路径、父目录跳转、越界的符号链接和无 manifest 的目录。包内路径继续按包根目录校验。首期不自动初始化 submodule 或获取 Git LFS 外部对象；若包依赖这些对象，作者必须将所需内容纳入可直接安装的发布内容。

安装只发布选定包目录；无需把整个仓库作为插件安装。获取可暂存仓库内容，如何减少传输量由获取所有者决定，不要求添加专门的服务。

### 4.2 预览、准备、提交与启用

1. 详情读取选定来源，解析 ref，返回具体 commit、子目录、manifest 摘要、兼容性及来源。此过程不运行包代码。
2. 安装入口接收明确来源；从详情安装时使用其已解析的 commit，不能重新解析同名 tag 后静默换包。直接命令安装则在操作内解析一次并报告结果。
3. PluginManager 获取该 commit 的包，在暂存目录校验 manifest、边界和兼容性，准备受支持的私有依赖。依赖安装需要解释器时等待 RuntimeManager 准备完成；无需这一步的包可以先完成安装，再等待 runtime 连接。
4. 准备成功后，PluginManager 按自己的原子记录契约发布包、保存实际来源和用户选择，再交给既有 loader。只有明确的安装、启用或更新动作可以请求 runtime；搜索与详情不触发获取。
5. 安装结果分别报告“包已安装”和 loader 的实际状态。RuntimeManager 的进度与失败由它的公开读入口取得；插件仍在等待服务时显示 Pending，不能显示 Active。

准备阶段失败或取消不改变已安装来源和选择；移除本次暂存内容。RuntimeManager 已成功取得的共享工具链属于它自己的记录，不因一次插件安装失败被删除。取消一个安装不能终止其他使用者已需要的共享 runtime。

记录提交是界线：提交之后调用方断开，不代表安装回滚。加载或重载失败必须报告已提交来源和失败原因，用户可显式重新获取先前的来源；不能保证运行中的旧代未被中断。旧目录清理与重载的在途工作影响仍由 [PluginManager](plugins/PluginManager.md#4-timing-and-conflicts) 和 rutis 契约决定，不增加历史安装备份机制。

### 4.3 更新、持久化与数据保留

更新使用同一个准备和提交路径。官方目录安装的插件在显式更新时采用当前二进制目录中对应的兼容引用；没有新候选则报告无更新，不追逐主仓库 HEAD。普通 Git 安装沿用记录的 ref；固定 tag/commit 不自动切换到其他版本，用户改变来源须是明确操作。目录项被删除时，旧包仍可运行和移除，官方目录更新返回无可用候选。

新增子目录和目录来源信息由 PluginManager 的安装记录持久化。写入这些字段前必须确定新的记录格式版本、旧记录读取、备份和旧二进制拒绝策略；不能复用旧格式号或丢弃字段后当作仓库根目录安装。具体版本号由实现时的当前格式决定，属于 [R1](#r1-pluginmanager) 的验收。

禁用、移除、来源变化及目录变化都不删除插件业务设置或历史记录。Workflow 的配置与运行记录仍由 [Workflow](plugins/Workflow.md) 拥有；本设计不迁移其格式。

## 5. Runtime 选择要求

RuntimeManager 提供三种来源策略：

| 策略 | 选择规则 |
| --- | --- |
| 本机优先 | 检查本机解释器及必要配套能力；满足宿主和候选包要求则使用，否则使用兼容的受管理固定版本，已有则复用、缺失则下载 |
| 显式 system | 只使用本机；缺失或不兼容时明确失败，不切换来源 |
| 显式 download | 使用宿主固定的受管理版本；不因 PATH 上存在解释器而改变选择 |

本机优先适用于 Node/Python，不能只检查命令是否存在。版本和必要工具检查由 RuntimeManager 负责，插件不能提交任意探测或安装命令。如果本机和宿主固定版本均不满足候选包约束，拒绝该候选，不下载任意版本，也不引入每插件 runtime 或依赖求解器。

已有就绪连接时，候选包先与该连接实际使用的解释器核对；不兼容则拒绝候选，不能为安装一个新包自动切换其他插件正在使用的解释器。用户显式调整 runtime 时，其对已启用插件的兼容性检查与错误须由 R2 规定。

一次获取选定来源后，记录实际解释器路径、版本和来源；当前连接代不因 PATH 变化被静默替换。重新获取时可按策略重新检查，切换依照 RuntimeManager 原有生命周期。只有缺失或能力不匹配触发本机优先的下载分支；已经选中的解释器启动失败、包安装失败或插件加载失败，分别报告实际失败，不通过换来源掩盖。

下载来源、版本固定、官方校验、解包边界和宿主运行时包继续归 [RuntimeManager](plugins/RuntimeManager.md#7-security)。系统解释器不被修改；私有依赖和受管理文件写入各自受控目录。保留配置策略与实际来源两种信息，读接口必须让用户看出本机优先最终用了什么。

启动是否获取仍由原有 `auto` 与已安装记录决定，不能把 `auto` 改义为来源策略。新发行配置可以选择本机优先，但必须保留用户已有的显式 system/download 选择；基础 CLI 新 home 未请求插件时不获取 runtime。

## 6. 客户端与服务边界

目录读取、详情和安装入口由 [#1173](https://github.com/arcships/dim/issues/1173) 在 PluginManager 的公开管理面实现，CLI、Desktop 和其他获准使用管理面的客户端共用它。客户端能力发现和入口授权沿用[管理方法契约](appserver/management-methods.md)，不能为浏览器复用而直接放开全部本地管理方法；Web 的管理接入需求由入口所有者单独闭环。

| 操作 | 输入 | 用户必须能看到的结果 |
| --- | --- | --- |
| 搜索 | 查询、来源过滤、分页位置与页大小 | 目录项、来源身份、读取时间、每个来源的可用性及下一页位置 |
| 详情 | 来源与目录项 ID，可选版本引用 | 具体 commit、子目录、包版本、内容种类、兼容判断及拒绝原因 |
| 安装 | 具体来源、可选安装 ID、是否启用 | 持久化结果、已安装 ID、实际来源、当前 loader 状态和可读失败信息 |
| 更新、禁用、移除 | 既有安装 ID 或名称 | 既有 PluginManager 操作结果；不另设目录安装状态 |

分页有界，游标无效时返回明确错误。一个来源失败允许带明确来源错误的部分结果；全部选定来源不可用则整次查询失败。详情对缺失条目、不可用 ref、不兼容包和无效路径分别说明原因。安装的获取、依赖、运行时与加载错误不混成“未找到插件”。具体字段和稳定错误码由 R1 的接口契约固定。

Agent 复用现有 `plugin` 工具，遵守其 Permission/Approval 规则；需要下一步使用的 ID、commit、路径选择和游标写入 `ToolResult.content`。搜索结果和仓库文档只是外部数据，不能自行授权安装或启用。配置来源的启用规则仍见 [ADR 003](../adr/003-approval-scope.md)。

### 6.1 CLI 交互示意

下列是目标交互，新增参数尚未发布。示例使用现有可执行名 `dimcode`；提供 `dim` 或 `dima` 入口的发行包采用相同参数与服务调用，不增加独立实现。

明确本地路径（`./workflow`、绝对路径）和 Git URL 保留原解释。短名同时匹配本地目录和目录项时拒绝歧义，要求使用明确路径或 `official:workflow`；社区可使用 `github:owner/repository`。不允许搜索排名或当前工作目录静默改变用户选择的来源。

```text
$ dimcode plugin search workflow
NAME              SOURCE      INSTALLED
workflow          Official    No
alice/workflow    GitHub      No

$ dimcode plugin inspect workflow
Source       https://github.com/arcships/dim.git
Revision     <compatible-tag> -> <commit>
Directory    runtime/plugins/workflow
Compatible   Yes

$ dimcode plugin install workflow
Runtime      Ready (managed Node; system Node incompatible)
Dependencies Installed
Package      Installed
Plugin       pending (waiting for runtime connection)

$ dimcode plugin list
workflow     running    Official    <commit>

$ dimcode plugin install https://github.com/team/plugins.git \
    --ref v1.2.0 --subdir plugins/echo
```

示例的 `running` 对应 loader 的 Active，不另造一套运行状态。社区结果不可用时在结果旁显示原因；空搜索显示无匹配项。下载失败时保留失败原因和可查询状态，不能把最后一行固定打印成成功。已有 `enable/disable/update/reload/remove` 的名称与状态归属不变；目录来源的更新规则见 §4.3。TUI 命令菜单顺序仍由 [CLI 菜单契约](cli-commands.md#command-menu) 决定。

## 7. 面向人和 Agent 的作者入口

复用[插件编写指南](../typescript-plugin-authoring.md)和现有 [plugin-authoring skill](../../runtime/plugins/plugin-authoring/skills/plugin-authoring/SKILL.md)，不再维护一套平行教程。指南拥有解释和公开契约链接，skill 提供任务步骤、必要检查和失败后的定位路径。

| 作者阶段 | 人类文档 | Agent skill |
| --- | --- | --- |
| 判断能力 | 包、服务、scope、生命周期和信任边界 | 读取宿主版本及已有公开服务，选最小实现，避免重复机制 |
| 编写 | 包目录、manifest、兼容声明、入口和私有依赖 | 使用匹配宿主的参考资料及同一个最小示例，不硬编码漂移的 SDK 版本 |
| 本地验证 | 在隔离 home 安装、调用一个真实功能、修改后重载 | 走相同公开流程，区分安装/运行时/加载失败，不触碰产品 home |
| 发布 | 发布可独立安装内容，选择 tag，添加 topic，检查目录详情 | 检查文件齐全及无源码 checkout 依赖；发布动作遵守用户授权 |
| 维护 | 兼容升级、显式更新、禁用/移除与业务数据保留 | 核对已有来源和版本，只更新任务要求的内容 |

指南和 skill 引用同一份可运行示例；示例随相同宿主版本验证，不各自复制实现。随产品交付的 skill 必须带齐必要参考文件，或链接到不可变且匹配宿主版本的公开文档；不得假设用户机器存在 Dim 仓库。没有匹配资料时明确指出缺口，不能用未发布 API 生成貌似可运行的插件。

类型脚本、纯内容包、Python runtime 和 UI 资源的能力分别按其实际 owner 契约教学；支持一种 runtime 不等于支持所有语言包管理器。本文的新分发步骤只有在对应公开入口可用后才能进入产品 skill 的执行步骤。作者文档和 skill 的交付应与分发能力一同验收。

## 8. Requirements raised to mechanism owners

本设计不修改其他所有者的合同；下列要求由所有者在各自文档和实现中承接。都不要求修改 Timeline、Dispatcher、事件信封、Session/branch 门面或 rutis。

### R1. PluginManager

- **受阻功能与合同**：[安装来源](plugins/PluginManager.md#22-pluginsjson)缺少包子目录与目录来源；[接口](plugins/PluginManager.md#6-interfaces-summary)不提供搜索和详情。
- **已有机制为何不足**：本地路径需要用户准备源码；仅仓库根目录的 Git 来源不能选择 monorepo 包；发行包 optional 来源要求先交付插件文件。
- **所需通用能力**：内置官方目录、GitHub topic 发现、可复用的详情解析、Git 子目录来源、兼容元数据与版本化安装记录。来源校验和安装仍由同一个 owner 完成。
- **顺序、批次、中断、重载**：不新增 Timeline 事件；明确准备/记录提交界线，保留原加载生命周期。取消、来源字段、缓存和失败语义按本文要求在 owner 文档具体化。
- **归属与 issue**：PluginManager，目录/管理面 [#1173](https://github.com/arcships/dim/issues/1173)，跨模块设计及来源扩展 [#1410](https://github.com/arcships/dim/issues/1410)。

### R2. RuntimeManager

- **受阻功能与合同**：[来源配置](plugins/RuntimeManager.md#22-configuration-the-runtimes-section-of-the-default-configuration-file)只有明确的 system/download，没有兼容本机优先。
- **已有机制为何不足**：默认下载忽略可用本机解释器；system 检查失败不能补齐缺失 runtime；复用 `auto` 会改变启动策略含义。
- **所需通用能力**：本机优先来源、宿主与候选包约束检查、实际选择的可观察结果，以及明确的获取触发和失败边界。保留显式配置，不求解任意版本。
- **顺序、批次、中断、重载**：不写 Timeline；来源在获取开始时决定，连接代内不漂移；共享获取的取消和配置/记录格式兼容由 owner 规定，不隐式覆盖旧配置。
- **归属与 issue**：RuntimeManager，[#1410](https://github.com/arcships/dim/issues/1410)。

### R3. Workflow 与发布所有者

- **受阻功能与合同**：[Workflow 安装入口](../../runtime/plugins/workflow/README.md#loading)需要源码准备；基础 CLI 不携带该包。
- **已有机制为何不足**：恢复默认打包会改变产品选择；仅增加目录项不会让包脱离 checkout 运行。
- **所需通用能力**：将现有子目录准备为自包含插件，加入官方 JSON 目录；发布时检查目录引用、包名、兼容元数据及生产依赖，不在安装器里判断 Workflow。
- **顺序、批次、中断、重载**：不改变 Workflow 事件、运行、配置和记录格式；通过既有插件路径验证安装、更新及移除。
- **归属与 issue**：Workflow/发布所有者，[#1091](https://github.com/arcships/dim/issues/1091)。

### R4. 客户端与作者资料

- **受阻功能与合同**：现有[管理入口](appserver/management-methods.md)和[作者指南](../typescript-plugin-authoring.md)不能展示或指导尚未定义的目录安装。
- **已有机制为何不足**：客户端自有目录/安装器会产生第二份状态；复制教程会使 SDK 版本和接口示例分叉。
- **所需通用能力**：客户端复用 R1 的数据接口；按第 7 节共同交付指南、skill、示例与版本匹配参考。浏览器管理授权不由本设计扩大。
- **顺序、批次、中断、重载**：界面只呈现 owner 状态；工具结果通过现有路径进入模型；无新 Timeline 事件。
- **归属与 issue**：CLI/客户端接线 [#1173](https://github.com/arcships/dim/issues/1173)，作者资料 [#1410](https://github.com/arcships/dim/issues/1410)。

### 8.1 服务面与影响面

| 入口或依赖 | Face | 所有者合同 |
| --- | --- | --- |
| `managementMethods` | `data`；新增目录操作仍为数据请求/回复，不传宿主句柄 | [管理方法](appserver/management-methods.md)、[服务面](service-faces.md) |
| PluginManager 的公开 `plugin.*` 与 RuntimeManager 的 `runtime.*` | 管理协议数据面；具体方法由各 owner 贡献，不能把协议方法当作新的 catalog 服务名 | [PluginManager §6](plugins/PluginManager.md#6-interfaces-summary)、[RuntimeManager §5](plugins/RuntimeManager.md#5-interfaces) |
| PluginManager 持有的 loader 控制句柄 | `internal(owner-api)`；不交给目录或客户端 | [rutis 使用契约](rutis-usage.md)、[当前 catalog](../architecture/rust-agent/generated/catalog.md) |
| RuntimeManager 的本地 owner API、解释器启动资源 | owner API 为 `internal(owner-api)`，进程资源为 `internal(host-resource)`；不宣称它们可跨语言注入 | [RuntimeManager](plugins/RuntimeManager.md)；R2 的公开请求只能携带数据 |
| Agent 的 `plugin` 工具 | 既有工具协议数据入口；不增加 Session 或 branch 服务 | [PluginManager §6](plugins/PluginManager.md#6-interfaces-summary) |

没有新增命名服务或 Capability，也不宣称现有管理插件 leaf-ready。实现若确实需要公开新服务，按[服务面契约](service-faces.md)定义数据面，不能把内部 Rust owner API 包装成第二套业务逻辑。

## 9. 可观察验收

验证使用临时 home 和可控来源；不访问产品 home。实现复用现有插件与 runtime 公共流程测试，增加下列缺失场景，不为私有 helper 新建测试体系。

| 场景 | 必须观察到的结果 |
| --- | --- |
| 离线搜索官方 Workflow；GitHub 限流 | 官方条目可见，社区来源明确不可用，未安装或获取 runtime |
| 社区同名包、伪造官方描述、无效根 manifest | 不覆盖官方身份；无效仓库不能成为可安装结果 |
| 预览后 tag 移动 | 安装仍使用预览选择的 commit，或明确拒绝不可获取内容，不换包 |
| 官方与显式 Git 子目录安装 | 获取正确目录；依赖不来自 Dim checkout；其他目录不作为插件发布 |
| 子目录越界、包内符号链接越界、缺文件或不兼容 | 安装在执行入口之前失败，原选择和业务数据不变 |
| 本机解释器兼容、不兼容和缺失 | 分别使用本机或受管理固定版本；显式 system 不降级下载 |
| 已就绪 runtime 与新包不兼容、短名与本地目录冲突 | 拒绝候选或歧义，不替换其他插件的解释器，不猜测安装来源 |
| 本机与固定版本均不兼容、校验或依赖安装失败 | 明确失败，不改系统环境，不激活候选 |
| 准备时取消与记录提交后断线 | 前者清理暂存且保留旧选择；后者可重新读取已提交结果 |
| 安装完成但 runtime/服务等待或加载失败 | 显示 Pending/失败原因，不能报告 Active；保留恢复所需来源 |
| 显式更新、目录变更、禁用与移除 | 来源选择可追溯，旧包准备失败不切换，插件设置和运行记录保留 |
| 新旧记录格式与旧二进制回退 | 按明确格式决定读取或拒绝，绝不丢失子目录而安装仓库根 |
| 干净环境按指南及 skill 创建同一示例 | 两条路径都能安装、调用、重载；不依赖作者 checkout 或未发布 API |
| 基础发行包与 Workflow 安装后功能 | 基础包仍无 Workflow；显式安装后既有 `workflow.*` 和工具可用 |

## 10. Rule check

| 规则 | 结论 |
| --- | --- |
| principles §1 | 无新 Capability；目录、安装和 runtime 均是 process owner 的状态，现有 Session 工具只调用公开操作 |
| §4.1 指令与结果 | 无分支业务状态变更；进程配置使用 owner 类型化操作，适用 §4.9 |
| §4.2 交接 | 不增加 Timeline 交接点；安装记录提交与加载之间的结果按第 4 节报告，不把断线解释为撤销 |
| §4.3 before | 不使用 before，也不在提交锁内执行网络、包管理器或插件代码 |
| §4.4 核心无业务含义 | 不修改 Timeline/Dispatcher/rutis，不增加 Workflow 特判 |
| §4.5 批次 | 无新 Timeline 批次；安装配置仍由 PluginManager 原子提交 |
| §4.6 模型上下文 | 只通过现有工具结果和已采用输入进入模型；目录不会主动注入上下文 |
| §4.7 公开交互 | 客户端与 Agent 不读取 owner 内部记录；服务面见 §8.1 |
| §4.8 基础设施 | 不向 rutis 提要求；现有 owner 的不足按 R1–R4 提出 |
| §4.9 必要事实 | 不将搜索缓存、下载进度或安装配置写入分支 Timeline |
| ADR 002 D1 | process 归属不变；不因客户端各自存在而复制安装状态 |
| ADR 002 D2 | 无目录适配插件、接线模块或第二个安装器 |
| ADR 002 D3 | 可选插件缺失不影响核心；索引项不构成加载依赖 |
| ADR 002 D4 | 安装选择与 runtime 配置不混入插件业务设置；业务数据保留 |
| ADR 002 D5 | 公共输入、回复与错误为数据，语言绑定放附录，内部资源不越界 |

## 附录 A：官方目录绑定示意

拟议文件 `runtime/official-plugins.json` 随源码维护，构建时嵌入。下面的 `<compatible-release-tag>` 是说明用占位符；实际发布条目必须为可解析的确定引用，不能携带占位符。

```json
{
  "format": 1,
  "plugins": [
    {
      "id": "workflow",
      "name": "workflow",
      "description": "Workflow definitions and runs",
      "source": {
        "git": "https://github.com/arcships/dim.git",
        "ref": "<compatible-release-tag>",
        "subdir": "runtime/plugins/workflow"
      }
    }
  ]
}
```

文件只描述可发现来源，不含 enable、off、loader 状态或用户业务设置。正式 schema 与源码绑定由 R1 确定；内置目录与用户安装记录是不同格式，不能共享版本号。加入条目不修改 `runtime/default-plugins.json`。
