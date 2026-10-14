# Rust Agent 设计规则

设计时的判断规则；机制与分工见[架构](../README.md)。事件与状态机的原则（只认序号、事件契约与消费规则、状态可恢复、写入前复核、副作用不重放）以[时间总线总纲](time-bus.md)为准，本文不重复。

## 1. Capability 有界

新功能做成一个 Capability。Capability 实现 rutis `TypedPlugin` 与 runtime `Capability`：`Deps` 声明全部依赖，含只用于装载顺序的依赖，`apply` 从 `Deps` 取得依赖；`apply` 注册服务与订阅，把句柄交给业务状态和回调；取得的资源立即用 `ctx.effect` 登记释放；`apply` 完成即服务与订阅可用，输入消费和后台业务在此之后启动。Capability 经 Session 级或 Timeline 级工厂注册，工厂只捕获配置和实例标识；作用域服务以该作用域的实例键（`SessionScope`/`BranchScope`）提供，依赖方以 `InSession`/`InBranch` 声明注入。Capability 之间的协调通过提交事件、订阅事件或 `Deps` 声明的服务完成。

所有可装配的运行能力和跨插件服务均由 Plugin 提供，包括文件访问、数据库和代码执行。普通模块只作为所属 Plugin 内部的函数、类型与算法实现，不能藏有绕过依赖声明的全局服务、资源或订阅。资源归属是单父树，服务依赖是允许多个上游的无环图；根、校验、失效重载以及唯一事件总线的目标约束见[单树装配](single-tree.md#产品装配约束目标)。

框架按 rutis 0.10.0 设计：使用 `TypeKey` 和 `require` / `require_as` 读取必需服务，使用 `EventKey<E>` 注册和派发事件。EventKey 的类型约束不代替 caps 事件声明（尚未实现，动态事件为 [ADR 001](../../adr/001-harness-foundation.md) 中的 Proposed）或产品授权，严格读取不代替依赖环预检。具体接口统一见[单树合同](single-tree.md#rutis)，不增第二套服务容器、事件总线或生命周期管理器。

## 2. 正交

加功能是新增 Capability，或扩展该 Capability 自己的行为和状态。需要改动其他 Capability 时，把协调改为事件或 `injects` 声明的服务，由各 Capability 按自己的消费逻辑处理。

## 3. 职责归属

| 职责 | 归属 |
| --- | --- |
| 事件事实与 sequence 分配 | Timeline |
| 事件含义、处理逻辑、写入时机 | Capability |
| Timeline 写入 | Capability，经分支内注册的 `Timeline` 服务 |
| 状态 | 拥有它的 Capability（`StateCell`，外部只读） |
| 异步操作与取消 | 启动它的 Capability |
| 事件处理器内的等待 | Capability 自持任务，登记后返回 |
| 模型、工具、上下文的执行与格式转换 | Capability 定义的宿主端口；审批宿主在 app-server |

职责放错位置时修正归属方。

## 4. 通过时间线扩展行为

新增或修改插件、能力、事件时，按以下规则设计；设计文档的“规则检查”一节逐条回答。

1. **要改别的模块拥有的状态，写一条指令事件。** 状态的所有者按序号读到指令后自己决定，把结果写成事件。提出要求的一方不写结果，不通过同步调用替所有者决定。例：`approval/request` 与 `approval/respond`；`input/withdraw` 与 `input/deleted`。
2. **对象交给下一个模块后不能撤回。** 交接要有明确的事件，例如 `input/applied` 把输入从 InputQueue 交给 Loop。交接前只有原模块能处理；交接后只能用新模块自己的指令，例如 `stop/interrupt`。
3. **要在事件写入前干预，只用 Timeline 的 before 回调。** 不在 RPC 入口、Session 入口或某个模块内部另加检查。before 回调可以修改要写入的事件、在同一批次追加事件，不能阻止写入。
4. **核心机制不带业务含义。** Timeline、Dispatcher、before、批次只提供通用能力；写什么事件、事件代表什么由插件决定。核心模块不为某个插件写特殊处理，只按自己公开的事件合同处理事件。
5. **对一个事件做决定前，先读完它所在的整个原子批次。** 同一批次里追加的修正（例如撤回）因此一定先生效。
6. **模型上下文只包含交接后被读取的事件。** 已采用并被消费的输入、工具结果、已采用的提示词才进入上下文。没有经过交接的东西（被删除的输入、被拦下的提交）不会进入上下文，不需要额外过滤。
7. **插件之间只通过公开的事件和服务交互。** 不读别的插件的内部状态，不同步调用别的插件替它做决定，不改别的模块拥有的状态。
8. **现有机制做不到时，向机制所有者提通用需求。** 机制所有者是运行时或 rutis。不在插件层加端口、检查点或适配器绕过。需求写明：
   - 哪个功能被哪条现有规则或合同挡住（引用条目）；
   - 考虑过哪些现有机制，各自为什么不行；
   - 需要的通用能力，不带业务含义；
   - 对序号、批次、中断、重载的影响；
   - 由谁实现，对应哪个 issue。

   已提出的例子：before 支持异步回调（运行时）；rutis link 断开时导入的服务保持登记、调用直接失败（#5）；运行时服务按名字暴露给非 Rust 能力（#4）。

插件放在哪个作用域、不设接线模块、配置分离、合同语言中立，见 [ADR 002](../../adr/002-plugin-scope.md) 的决策。

## 5. 验证

需求变化时，旧断言按新需求修改或删除，测试跟随实现。日常行为验证用各 crate 的 `crates/*/tests` 与仓库根的 `tests/integration`（跨包扩展、公开流程）。事件格式样本位于 `crates/dim-agent-types/tests/fixtures/`，是冻结的兼容基线：保留原始公开 wire 样本及其来源，不随实现变更更新；契约测试（`crates/dim-agent-types/tests/*_contract.rs`）对这些样本做反序列化与序列化校验，`cargo run -p dim-agent-types --example compat -- --emit` 输出原生 Rust 样本的 v1 wire 形式供对照。对冻结事件结构的扩展须经设计决定，并在契约测试中登记。
