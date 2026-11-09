# Prompt：保存有效条目身份，按身份读取内容

Status: Draft.
Scope: branch Prompt 状态、采用流程与恢复。
Issue: [#1320](https://github.com/arcships/dim/issues/1320)。
依据：[Prompt](plugins/Prompt.md)、[模型配置](model-configuration.md)、[数组查询](timeline-array-rows.md)、[状态结果](owner-state-indexed-reads.md)、[设计规则](../architecture/rust-agent/principles.md)、[ADR 002](../adr/002-plugin-scope.md)、[插件合同](plugin-contract.md)、[rutis](rutis-usage.md)。

## 1. 决定

Prompt 保存当前有效条目的身份，不在每次打开或采用时重放 prompt/changed、turn 和 scope 历史。条目正文仍留在原 changed 事件；通过 query 的数组元素行按事件 ID 和原下标读取。

保留 PromptSnapshot、原配置采用事件和原公开接口。新增 prompt-state/recovered 结果，按状态结果协议确认已处理输入。不增加 Prompt Reader、通用 KV 或数据库依赖。

## 2. 恢复对象

state 的唯一类型 PromptRecoveryState：

| 字段 | 含义 |
| --- | --- |
| entries | 当前有效键到 PromptEntryReference 的映射，键为 source 与完整 scope。 |
| turns / tasks | 当前活动身份，不保存已结束生命周期。 |
| revision | 最后采用的 PromptReference，包括仅 Revoke 的变更。 |
| system_revision | 原状态机定义的 System 版本，不以存活条目的最大版本代替。 |
| configuration_event | 最后已采用 ContextConfiguredData 的事件 ID 或 null。 |

PromptEntryReference 含 source、scope、event_id、element_index。最后两个字段定位 Set 操作中的完整 PromptEntry，不复制 body。recovered 不保存历史 changes 或另一份拼接 System；已有 context/configured 继续提供已采用的 System 和 source text。

Branch 当前 StateCell 仍持有实际 PromptSnapshot，服务现有采用比较和通知。恢复记录使用引用只是持久化表达，不引入第二套常驻条目表。状态与位置在同一 owner 更新，不能从不同锁拼出组合。

源位置只使用结果协议的 source_through，不在 state 再保存一个同义水位。

## 3. 变化时处理

| 源事实 | 处理 |
| --- | --- |
| prompt/changed | 按 changes 原下标顺序应用；有效 Set 替换引用，Revoke 移除引用；保留原 revision/System 规则。 |
| turn/start、turn/end | 更新活动 turn，结束时移除属于它的条目。 |
| context/scope | 更新 task 活动性；结束清除该生命周期的条目，后续重启不复活旧 Set。 |
| branch/start | 沿用原 turn 清理与祖先可见性。 |
| context/configured | 更新配置事件身份和原 Snapshot 配置。 |

条目失效在 owner 处理该源事实时确定，不交给 SQL 重新解释所有生命周期。Set 时不活动的 scope 不登记引用。结果与当次必需退役输入同批提交；退役输出仍经过原采用入口。

## 4. 冷恢复查询

```text
最后 PromptRecoveryState
  -> 按 configuration_event 查询配置
  -> 当前 entry 引用按最多 128 项分批
  -> query 展开指定 changed 事件的 changes
  -> 只选择指定原下标的 Set.entry
  -> 得到当前 PromptSnapshot
  -> 处理索引中的真实未完成源输入
```

元素请求使用 rows=elements、field=data.changes；筛选是事件 ID 与该事件的下标集合，返回 position、element.index、element.value.entry。不同事件的 ID/下标必须成对匹配，不能使用两个独立 In 产生交叉组合。正文在持久化边界解码为唯一 PromptEntry 类型，后续消费者直接使用。

有 K 条当前有效提示就读取 K 条实际内容。SQL 必须先按 ID 和下标定位，不按所有历史 source/scope 分组。一个旧事件还保存其它大元素时，单纯 json_extract 仍可能解析整个 JSON；数组存储必须提供元素定位或独立内容引用才能保证正文成本。不把 Rust 只返回一项当作 SQLite 没处理其余数组。

因此数组接口首版在此负责“按已知身份取元素”，不要求同时增加计算字段、Set/Revoke 路径别名或复杂生命周期 SQL。旧数据初始化才使用历史元素操作来建立引用。

## 5. 收拢五个读取入口

| 入口 | 修改 |
| --- | --- |
| Prompt apply 的初始投影 | 只做上节恢复一次，发布同一 StateCell 与源位置。 |
| Prompt open | 在处理 opening 时固定其 Snapshot/引用，不在异步 load 后读取已经前进的当前值冒充旧前缀。 |
| configure-result 采用 | 使用 owner 同一采用任务中的当前已提交状态。 |
| ConfigurationPublisher.publish | 接收明确的已采用状态与配置，执行真实配置转换和原批次写入；不再自己恢复 Prompt 历史。 |
| ConfigurationPublisher.adopt_guide | 先按配置身份判断；无配置直接结束。有配置时使用其 System/source 和 owner 的 System 版本。 |

ConfigurationPublisher 不反向依赖 Prompt StateCell，避免加载环。当前状态由 Prompt 作为显式类型参数传入；不新建读取包装。发布后 owner 处理实际提交的事件，不能只应用变换前草稿。工具采用在 Dispatcher 尚未启动的装配阶段也不等待 Dispatcher：沿用同步调用的恢复/采用步骤推进已提交状态，后续派发按源确认跳过。普通输入任务仍等 activated，closing 不启动新的采用。

同一 owner 的采用串行；若 load 完成时源状态或 scope 已变化，用当前有效状态比较候选，不重放历史。写入前的竞争按已有提交注册检查处理，不能拿旧 Snapshot 覆盖新配置。

## 6. 持久化和机制需求

| owner / 问题 | 当前不足 | 需要的能力 | 位置、提交、取消 |
| --- | --- | --- | --- |
| Timeline / #1320 | 数组投影不能逐元素返回与分页 | 数组行、原元素下标和按成对身份选取；不需要任意计算字段 | 原 through、祖先截止和缺失值合同不变。 |
| Store / #1320 | 大 JSON 选一个元素仍可能解析完整数组 | 按事件身份/元素下标定位公开数组内容；与成员索引一同作格式决定 | 既有完整事件读取仍还原完整数组，不能修改事件 wire fixture。 |
| Store / #1320 | 恢复结果确认关系目前无直接成员执行 | 对上述源事实与 prompt-state/recovered.source_ids 建立精确索引 | 结果与确认同事务；普通恢复不建索引。 |

元素内容可沿用 Store 内容节点，但插件只持有事件 ID 和数组下标，不持有内容哈希。该格式要求在存储 owner 的[下一格式提案](appserver/native-storage-format.md#提案状态读取索引与数组元素)决定，本文不指定另一套私有 SQL 表。

## 7. 验收、Rule check 与删减

复用 Prompt 配置、guide、scope、工具采用和请求 System 流程；覆盖同事件 Set/Revoke 顺序、已结束 scope 重启、仅 Revoke 的 revision、fork 截止、overlapping load、Dispatcher 未启动的工具采用、关闭竞争与结果提交各阶段崩溃。

固定当前 K、未完成 U，增加十倍旧变更和结束 scope，恢复仍只取结果、K 个有效元素和 U 条真实工作。另扩大单个历史数组，验证按下标选一个元素的实际解码字节。记录结果写入 O(K) 的身份元数据，不复制 K 份 body。

| 规则 | 本设计 |
| --- | --- |
| principles §1；ADR 002 D1 | 状态和采用仍归 Prompt branch，guide 表仍是原 Session owner。 |
| ADR 002 D2–D5 | 无接线层、新服务或配置；引用为语言中立身份。 |
| §4 规则 1、2 | 加载不等于采用，不替其它 owner 交接对象。 |
| §4 规则 3、4 | 不在 before 恢复，不让 SQL 解释 Prompt。 |
| §4 规则 5、6 | 既有配置/上下文输入批次不变，确认随结果提交。 |
| §4 规则 7、8 | 继续用声明服务，数组与索引缺口交机制 owner。 |
| §4 规则 9 | 结果用于未来采用和恢复，不是 UI 日志。 |

删除五处各自重放，不增加五个 Reader。选择保存当前引用，删掉通用历史按键分组和生命周期 SQL 的前置要求；保留元素行能力是为了取当前引用的原始内容。
