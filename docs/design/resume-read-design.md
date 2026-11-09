# Resume：按业务对象设计计算与读取

Status: Draft.
Scope: CLI resume 的跨模块流程；不定义各所有者的内部状态。
相关问题：[#1320](https://github.com/arcships/dim/issues/1320)、[#1147](https://github.com/arcships/dim/issues/1147)。

本文组织已确认方向的具体设计，不表示提案接口已经存在。保留 append/query 入口；原子批次用于结果与必需输出，成员索引用于待处理和 pending。持久化格式与新事件按相应 owner 的提案实施，不扩大既有 issue 的已约定范围。

正常状态读取的约束见[状态结果与未处理输入](owner-state-indexed-reads.md)：禁止扫描历史后筛选或 Fold。结果提交前崩溃留下真实未处理输入，恢复只处理该集合。原始历史重建只用于初始化、显式修复与等价对照。

## 1. 先确定结果，再确定读取

| 业务 | 要取得的对象 | 计算与 I/O 设计 |
| --- | --- | --- |
| 上下文占用 | 当前贡献总量、算法、基准和模型预算 | [ContextUsage](plugins/ContextUsage.md)：变化时计算，数字节点和结果同批保存，查询不重建请求。 |
| 输入队列恢复 | entries、turn、starting 和真实欠写结果 | [InputQueue 恢复](input-queue-recovery.md)：读取保存的控制状态，不从 UI pending 推导。 |
| Prompt 恢复 | 当前条目引用、活动 scope、配置身份与版本 | [Prompt 状态读取](prompt-state-read.md)：保存有效引用，按事件 ID/元素下标取内容，删除五处历史重放。 |
| 首屏与折叠详情 | 有界显示窗口，以及用户展开的详情 | [Session 显示读取](appserver/session-display-read.md)：先选择显示对象，正文按需读；显示对象与完整事件明确分型。 |
| pending / turn | 各所有者当前未完成集合、轮次及输出水位 | 保留[输入状态读取](input-state-read.md)的结果语义；[Store 成员索引](appserver/timeline-member-index.md)直接分页集合。 |
| Goal | 当前 Goal 状态、执行控制和源位置 | [Goal 状态读取](plugins/Goal-state-read.md)：保留原模型可见状态，读取 owner 完成结果，删除补偿先作为工作结算。 |
| SQLite 属性执行 | 调用方指定的字段或计算结果 | [读取执行补充设计](timeline-read-followups.md#属性投影与累计执行成本)：批量投影，统计关联和内部补查成本。 |
| 首开完整性校验 | 存储合同要求验证的记录及关系 | [恢复读取成本](appserver/restore-read-cost.md)：保留必要扫描，单独测量并减少重复处理。 |

三类工作不能互相替代：SQL 判断存在性、关联和最新值；业务所有者解释状态变化；正文读取服务显示或本次模型请求。分页只是分批返回，不改变应计算哪些对象。

## 2. 目标流程

以下是目标流程，不是当前装配图。各部件的服务、订阅和恢复由其设计定义。

```text
attach
  -> 取得或恢复 Session：校验与各 owner 恢复分别计时
  -> 固定显示窗口位置
  -> 读取显示窗口并投影，尽早显示
  -> 安装相同截止位置的 owner inventories，交接后续事实

usage 读取
  -> ContextUsage 的指定位置结果
  -> 不阻塞 CLI 的历史回复、发布和用户命令处理

展开详情 -> 按身份读取详情
加载更早 -> 在原窗口范围继续分页
```

窗口投影可以先显示；依赖 owner inventory 的操作在该 inventory 可用前不得使用历史窗口推断出的替代状态。查询失败保留错误与重试能力，不作为空集合或无运行任务。

## 3. 控制状态读取的业务边界

pending 方法分别返回各自未结束对象。候选和 terminal 关联使用同一截止位置；续页只推进外层结果，不能排除游标之前的 terminal。正文只在实际展示或结算该对象需要时选择。

当前轮次与未完成输出的绝对水位来自 owner 方法。输出字符计数的含义是 Unicode 字符，不是 usage 的序列化字节统计，不能互相代用。

Goal 已有完整状态记录，但删除提交后、补偿提交前，该记录可能不代表正确状态。正常读取必须确认记录有效性；候选 fold 只用于迁移、构建和等价对照，不作为隐蔽回退。只读当前状态不意味着丢掉配对、旧格式和删除补偿语义，也不预先要求任意历史状态版本接口。

普通未打开分支不读聊天正文。不得为简化 UI 调度新增一个读取所有分支完整状态与历史的聚合服务。独立 inventory 的网络等待若经测量成为瓶颈，可在固定截止位置并发不同方法；同一方法的续页依赖仍按序。

## 4. 成本如何验收

| 路径 | 必须减少或界定的工作 | 不能据此宣称解决的问题 |
| --- | --- | --- |
| usage 正常读取 | 读取持久统计，不读已结算消息正文；在线查询不重建历史 | 异步请求本身不降低服务端 CPU；持久化方案必须另外证明一致性。 |
| 输入、Prompt 恢复 | owner 接收的字段和处理对象有明确后续用途 | 返回当前对象不保证 SQL 没扫全部 origin 或 change。 |
| 显示窗口 | 隐藏请求正文加载为零；折叠详情在展开前不传输 | 事件条数不能代表字节上限；单个原子批次可能很大。 |
| 完整性校验 | 明确记录数、解码及关系检查成本 | 它的必要全量扫描不能作为其它模块重放历史的理由。 |

记录产品装配下的校验、owner 恢复、插件 opening、历史选择/解码/编码、客户端投影/首帧、inventory 和 usage。不同取样范围的耗时不直接相加。

分别扩大已结束历史、当前未完成对象和单条内容大小。记录 wall time、CPU、Rust 与 SQLite 内存、读取正文和返回字节、SQL 总指令及额外写入；同时检查首个页面和全部页面累计。已有有效统计的恢复与首次初始化必须分开报告。

## 5. 设计约束与删减

不新增公共 Reader、完整历史缓存或替各模块计算业务状态的统一恢复层。ContextUsage 的独立部件由 [ADR 023](../adr/023-context-usage.md) 决定；其它业务继续由原 owner 处理。

各草案明确区分可复用的现有机制与尚缺的能力。没有证明等价的直接查询，不取代现有恢复算法；没有一致提交保证的异步快照，不宣称正常冷恢复无需补算。设计不得通过改变 UI 数值含义、跳过校验或删去恢复义务获得性能收益。

## 6. 最终 API 与模块关系

以下为目标调用关系，不是生成的当前装配图：

```text
CLI / ACP / Web
  -> 原 session.* 方法
     -> 对应业务 owner：返回已完成结果、组织字段查询
        -> Timeline.query / TreeCommands.query
           -> Store：SQL、成员索引、数组内容定位

业务 owner 的输出 / 恢复结果
  -> 既有 append / 原子批次
     -> Store 同事务维护索引
        -> 提交后原样发布与派发
```

| 入口 | 决定 |
| --- | --- |
| session.goal.get / session.context.usage | 保留；不在方法内重建历史。 |
| session.input/question/exec/collaboration.pending | 保留参数、返回和 through；只替换 Store 执行方式。 |
| Timeline.query / TreeCommands.query | 保留；只增加[元素行](timeline-array-rows.md)这一项表达能力。 |
| append 与原子批次 | 沿用事实写入；不增加 getState、saveState、SQL 或事务服务。 |
| feature 内部 contextUsage.read | 业务统计接口，ContextPreparation 消费；不是新的通用读取层。 |

四个 owner 的新恢复类型分别为 goal/recovered、input-queue/recovered、prompt-state/recovered、context-usage/recovered。它们各自定义 state，公共位置/确认语义只在[状态结果合同](owner-state-indexed-reads.md)定义。结果不是另一份模型上下文。

这些是新增事实合同，不是无成本的缓存。恢复结果增加写入与历史保留；如果缺少源成员索引，仅增加这些事实不满足本设计，不能以更名快照代替执行优化。

本次首先改造现有 Rust owner，复用已有 Rust 原子批次。跨语言 timeline 当前没有暴露同等的任意多事实批次，不能宣称 TypeScript 已支持本结果协议。今后扩展该绑定时应暴露既有原子批次能力，另列公开合同变更；不能用多次 append 冒充原子提交，也不为此增建状态数据库 API。

## 7. 一致性与初始化流程

```text
原事实提交
  -> 索引中出现待处理源身份
  -> owner 处理真实工作
  -> [必要输出、恢复状态、此次源 ID 确认] 一次提交
  -> 待处理源从索引移除，发布结果

resume
  -> 读最后兼容结果
  -> 索引取未完成工作，空则不读旧输入
  -> 处理完后交接实时消费
```

源输入截止与结果读头不同：结果可以晚于被处理输入提交。恢复通过一次 query 固定确认读头，在外层限定源范围，不能让新完成确认因原源截止而不可见。普通 pending 保留原同一 through 的事实语义，不采用这条处理确认规则。

未配置统计、没有 Goal、空队列是各自明确结果，不触发通用恢复。缺结果、schema 升级或首次启用 owner 是初始化：一次构建状态与基线，普通 resume 不反复做。缺节点/索引是错误，不是“没有状态”；初始化和错误不能悄悄降级成全历史查询。

## 8. 实施顺序与文件

| 顺序 | 修改对象 | 完成条件 |
| --- | --- | --- |
| 1 | storage/query.rs | 修 FTS 驱动方向、删除关系方向、批量字段投影，原语义对照通过。 |
| 2 | runtime 查询类型、Memory、SQLite、声明绑定 | 元素行按成对身份取值，旧事件查询不变。 |
| 3 | storage/timeline.rs、sqlite.rs、codec 与格式升级 | 成员根、数组元素和结果位置索引可用，事实/索引原子且支持旧 through。 |
| 4 | InputQueue | 完整控制状态与终态同批保存，恢复不再八类型重放。 |
| 5 | Prompt / ConfigurationPublisher | 引用恢复及单一 owner 状态；五个旧 read 入口替换。 |
| 6 | Goal | 完成结果与未处理工作恢复，get 删除候选 fold；删除路径单列实际受影响工作。 |
| 7 | ContextUsage / context_model / 请求事实 owner | 数字状态、算法合同、基准引用和统计查询，删除 usage 的 rendered_stream。 |
| 8 | CLI / ACP、显示窗口 | 首屏不等 usage，采用原方法安装 inventories；展开详情才取大正文。 |

存储新增结构和数组编码必须采用[下一格式提案](appserver/native-storage-format.md#提案状态读取索引与数组元素)，不能无声改 v36。数组/索引就绪前不把 owner 的目标恢复写成已完成性能改造。

## 9. 读写量验收

N 为已经处理的历史，K 为当前结果成员，U 为真实未完成工作；这些数量分别变化：

| 路径 | 目标读取 | 新增写入 |
| --- | --- | --- |
| Goal get | 一个完成结果中的 goal 字段 | get 不写；业务变化写结果及原必需输出。 |
| pending 一页 | 一个版本根、树路径和本页对象 | 原事实提交维护成员路径。 |
| InputQueue 恢复 | 当前 K 项控制状态，加 U 项工作 | 本批状态与输出；首版状态序列化随 K 增长。 |
| Prompt 恢复 | 当前引用元数据、K 个元素、U 项工作 | 当前引用状态与必要采用事实，不复制历史 body。 |
| usage | 当前总量、限制和适用基准，不展开消息叶 | 新增/替换的数字路径、结果；无逐 delta usage 日志。 |

历史 N 扩大十倍、K/U 不变，SQL 不得出现十倍 origin 扫描。只读一个结果不代表写入免费：同时量 result 字节、数字/成员节点、WAL、FTS 与序列化成本。新设计未实测前不引用旧标量快照微测来承诺整个产品耗时。

默认身份页为 128，当前内容按需分批读取。行数不限制单对象字节；Goal 的 JSON 结果还可能包含实际未完成执行状态，必须记录 SQLite 解析的结果体积，不能把选择一个字段说成只读了这个字段的字节。

验证范围包括源事实与结果之间崩溃、批次提交后回调丢失、并发读取/续页、schema 初始化、分叉切点和 self-output 顺序。最后用与 #1320 相同口径拆分 attach、owner 恢复、首屏、inventories 和 usage，不把不同样本相加报总收益。
