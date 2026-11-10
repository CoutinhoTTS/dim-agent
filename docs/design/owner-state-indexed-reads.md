# 状态结果与未处理输入

Status: Draft.
Scope: 状态 owner 的恢复结果、未处理工作与 Timeline 的读写关系。
Issue: [#1320](https://github.com/arcships/dim/issues/1320)。
依据：[查询合同](timeline-property-query.md)、[插件合同](plugin-contract.md)、[ADR 023](../adr/023-context-usage.md)。

## 1. 决定

每个 owner 按源事实的原始顺序处理输入，并保存“已经处理到哪里”和完整恢复状态。正常恢复先读这条结果，再用位置索引读取它之后的相关输入。

写入沿用 append 和既有原子批次；读取沿用 query。插件不访问数据库。没有通用状态 CRUD、Reader 或恢复协调插件。

本协议不允许乱序完成输入，因此不需要逐输入确认数组或“未处理源成员”索引。公共 pending 是另一个问题：一个任务可以先开始、很晚才结束，仍使用[成员索引](appserver/timeline-member-index.md)。

## 2. 保存对象

恢复结果采用各 owner 自己的事实类型。源事实和模型可见输出仍保留原合同；恢复结果不进入模型上下文。

| 字段 | 含义 |
| --- | --- |
| schema | owner 的恢复状态版本；与数据库格式、RPC 版本分开。 |
| source_through | 此位置及以前、属于该 owner 的源事实已经全部处理，且必要输出已提交。 |
| state | 这个位置的完整恢复状态，或该 owner 不可变状态节点的身份。 |

结果自身的 ID 和 sequence 来自原事件信封。它的提交位置不是 source_through：处理源事实 100 后，可以在位置 105 写结果，不能据此宣布 101–104 也已处理。

首版 schema 为 goal/v1、input-queue/v1、prompt-state/v1、context-usage/v1。state 的类型和源事实条件由各 owner 的设计唯一规定。当前状态只定义一次，不在内存和恢复记录中各造一套业务模型。

一个未完成的业务任务可以保存在 state 中。例如等待模型回复的请求仍未结束，但 owner 已处理“请求开始”这条输入。source_through 确认输入已处理，不宣布这个业务任务已结束。

## 3. 写入

```text
源事实提交 -> Dispatcher 按位置送达 owner
             -> owner 按序计算下一状态
             -> 原子提交：[本次必要输出，新恢复结果]
             -> 提交成功后发布新状态
```

一批可以处理多个源输入，但必须保持原顺序。只有这些输入的转移和必要输出全部完成，才能把 source_through 推进到本批最后源位置。不能让后一个输入先完成，再用它的位置跳过前一个输入。

源事实提交后、结果提交前崩溃，旧结果的 source_through 未变；恢复仍会读到这些源输入。批次提交后崩溃，输出和新结果同时存在；恢复不再重复输出。

owner 自写且也属于自己源范围的事实，统一作为后续输入按序消费。写结果时不把这些尚未消费的输出位置算入 source_through。若本次已决定并提交输出，state 保存处理它所需的去重身份；具体字段由 owner 定义。不能过滤全部 self-origin，也不能凭结果自身的位置跳过中间事实。

输入不改变业务状态时，仍须推进处理位置。每批最多 128 条源输入；达到上限或当前已接受队列读空时，合并写一次结果。不能只因为 state 未变，就无限期丢弃处理进度。关闭前保存已完成进度。崩溃后只重做尚未提交进度的那一批，不能把已经保存的前缀重新处理一遍。

提交失败走现有 fault 路径，不发布未提交状态，不在这份状态上继续接受下一批。纯内存转移和同步 append 可在 handler 完成；需要等待的工作由 owner 自己管理顺序、任务、取消和关闭，不在 handler 等待数据库、网络或工具。

## 4. 冷恢复

先固定本次恢复的源截止 S。同步处理事实的 owner 沿用 Dispatcher consume 交接。[ContextUsage](plugins/ContextUsage.md#10-源输入写入及恢复) 的异步任务则把既有 Dispatcher 订阅仅作位置通知，统一从已保存前缀读取；不把回调到达当作处理完成，不持有第二份事实队列。两种方式都使用同一 source_through，先处理固定恢复范围，再处理更晚的源。

1. 在当前可见读头内，用结果类型和位置索引读最后一条本地结果。
2. 检查 schema，读取 state 和 source_through。正常恢复要求 source_through 不超过 S。
3. query 只读 `source_through < 源位置 <= S` 中该 owner 的相关事实，按原位置处理；每页最多 128 个身份。
4. 同批保存必要输出和新结果，推进 source_through；完成到 S 后交接 S 之后的实时输入。

第 3 步读的是尚未保存处理结果的真实工作，不是扫描历史后排除已处理记录。没有新相关输入时，一次索引空查询就结束，不读旧源正文。范围中有 U 条相关输入，就处理 U 条；历史中已完成的 N 条不再参与。

只有实际计算需要内容的源，才按身份读取相应正文。大正文逐对象消费并释放；128 仅限制身份数量，不是正文大小。get 不执行这些恢复工作：它等 owner 完成本次固定范围的处理，或沿原合同返回未就绪，不能把未就绪伪装成空状态。

结果限于本地 opening 之后。fork 的复制前缀、祖先引用、旧库和首次启用 owner，由该 owner 显式初始化；不能把复制来的结果直接当成新分支已经消费输入的证明。初始化完成后只处理新分支的源输入，固定祖先不重新派发。

## 5. Query 请求形状

下表使用既有 QuerySpec 的名字，不新增方法。结果查询的读头可以晚于 S，因为结果可能在源输入之后提交；源读取仍固定在 S。

| 请求 | 范围与条件 | 返回 |
| --- | --- | --- |
| 最后结果 | Branch；本地 opening 之后；kind = 本 owner 的结果类型；Last，limit=1 | data.schema、data.source_through、data.state。 |
| 未处理身份 | Branch；from = source_through + 1，through = S；kind 为 owner 声明的源类型，附其公开属性条件；位置升序，limit=128 | id、kind、position，以及 owner 必需的控制字段。 |
| 实际工作正文 | 同一固定源范围；id = 本次需要正文的源 ID；limit=1 | 本次计算需要的属性；用完释放。 |

若 source_through 已等于 S，不再查询空后缀。否则首个源查询固定 through=S，续页保持相同范围和游标。页提交成功后可以继续处理原固定页；不能把新读头与旧游标混用。

源查询没有确认关系、Not Exists 或 Any(source_ids)。位置下界直接进入索引范围，不能先查 owner 全部历史类型，再在 Rust 中比较 source_through。多个源类型允许各自按位置索引定位，再合并这 U 条新输入；不得枚举旧前缀。

最后结果先取最新记录，再检查 schema；不在 filter 中排除未知 schema 后冒用更旧结果。已打开且处理完成的 owner 直接返回同一已提交状态，不为每次 getter 强制读取数据库。结束分支无 owner 时沿既有 TreeCommands.query 读取最终结果，不新增历史状态版本接口。

## 6. 初始化和错误

旧数据没有结果，按原业务语义显式构建一次，生成固定截止的 state 与 source_through。构建失败不发布结果。后续恢复不能再走旧历史算法。

最新结果是已知可升级 schema 时，按 owner 的升级设计初始化；较新或未知 schema 返回不兼容。结果损坏、节点缺失和位置越界返回错误，不偷偷使用旧结果或全历史重放。索引缺失由 Store 打开/升级合同处理，普通 query 不代建索引。

首次创建的空分支可以使用默认初态；不为零工作制造结果循环。带继承历史的分支必须先完成其业务初始化。

## 7. 成本、验收和删减

| 路径 | 允许的工作 |
| --- | --- |
| 已就绪 get | 返回 owner 当前状态；无 owner 时定位一个结果及实际返回字段。 |
| 无未处理输入的恢复 | 一个结果定位、当前必要状态节点，至多一次空源查询。 |
| 有 U 条未处理输入的恢复 | 按位置索引定位 U 条输入，处理它们所需的新内容。 |
| 普通变化 | 计算新增或受影响状态，同批保存必要输出与一次合并结果。 |

例：已保存到 99，输入 100 触发 applied。结果与 applied 同批提交，source_through=100。即使结果位于 105，另一个尚未处理的源输入 101 也不会被跳过。提交前崩溃重读 100；提交后恢复从 101 开始，不重复 applied。

验收覆盖提交前后崩溃、回调丢失、同批多输入、self-output 中间夹入外部输入、重复恢复、关闭、fork 和 schema 升级。固定 U 和当前状态，增加十倍已完成历史，源查询必须出现位置上下界，不增加旧源扫描和正文读取。

删去逐输入确认数组、确认关联表和恢复源成员树：按序完成与 source_through 已经表示完整已处理前缀。保留公共 pending 成员树是为定位乱序结束的业务对象；保留状态结果是为不重做已经完成的计算。两者不能混为一种查询。

## 8. 查询绑定示例

以下 JSON 使用现有 TimelineQuery / QuerySpec 的字段。调用方已经选定 Session 和分支；请求内不重复传 Session ID。示例分支为 main，本次源截止为 10,000。结果类型和 state 字段仍由对应 owner 定义，不在通用 query 中增加 owner 名称或状态枚举。

### 8.1 读取最后结果

假定本地 opening 位于 40，当前可见读头为 10,005：

```json
{
  "scope": "branch",
  "from": {"branch_id": "main", "sequence": 41},
  "through": 10005,
  "include_publishing": false,
  "query": {
    "filter": {
      "op": "in",
      "value": {"kind": "field", "path": "kind"},
      "values": ["goal/recovered"]
    },
    "select": {
      "kind": "fields",
      "fields": ["data.schema", "data.source_through", "data.state"]
    },
    "group_by": [],
    "pick": "last",
    "order_by": []
  },
  "page": {"limit": 1, "after": null}
}
```

查询只选一条结果。owner 检查这条记录的 schema 和状态；不添加 schema 条件去寻找更旧的兼容记录。若其 source_through 为 9,980，下一步从 9,981 查询源事实，不能从结果自身的提交位置继续。

### 8.2 读取未处理输入的身份

下面以 ContextUsage 的 input/consumed 源为例。其它源类型使用其设计声明的完整类型集合，不把该示例的单一类型当作全部统计输入。

```json
{
  "scope": "branch",
  "from": {"branch_id": "main", "sequence": 9981},
  "through": 10000,
  "include_publishing": false,
  "query": {
    "filter": {
      "op": "in",
      "value": {"kind": "field", "path": "kind"},
      "values": ["input/consumed"]
    },
    "select": {
      "kind": "fields",
      "fields": ["id", "kind", "position", "data.input_id"]
    },
    "group_by": [],
    "pick": null,
    "order_by": [{"key": {"kind": "position"}, "direction": "ascending"}]
  },
  "page": {"limit": 128, "after": null}
}
```

只有回复包含 next 才续页；续页保持原请求，只把 after 改为该 next。不能把读取到的最后一条事件位置当成新的业务截止，也不能在分页之间更新 through。

### 8.3 已知身份的正文与节点读取

收到 input/consumed 后，其 input_id 指向的 input/message 可能早于 9,981。点查这个输入使用原业务可见范围，而不是未处理源的窄范围：

| 参数 | 取值 |
| --- | --- |
| scope | 输入允许来自祖先时用 chain；只允许本地时用 branch。 |
| from | null；不能沿用源后缀的 9,981。 |
| through | 同一次业务处理的固定截止 10,000。 |
| filter | In(Field("id"), [input_id])。 |
| select | Fields；只列实际计数需要的 blocks、user 等属性，由 ContextUsage 的投影合同确定。 |
| page | limit=1，after=null。 |

这里的 In / Field / Fields 是上述 JSON 结构的缩写，不是新 API。Store 先按身份定位并检查范围，再读取正文；不能为了找到一个 ID 扫描类型历史。数字或成员节点同样按 ID 批查，每批最多 128 个；读取范围须包含引用实际所属的祖先切点，不能凭 ID 绕过可见范围。

Fields 返回 QueryRow / QueryValue，不构造缺少字段的 AgentEvent。只有业务确实需要完整事件时才使用 Events；返回正文用完即释放，不放入共享 Reader。

### 8.4 仅判断是否有新源

结束分支的 getter 只需确认结果之后是否还有源时，沿用 §8.2 的固定范围和源条件，将 select 改为 `{"kind":"exists"}`，清空 order_by，page 设为 null。调用方只使用 exists；不取身份页再检查非空。

Exists 只证明是否有新源，不计算新状态。存在新源时按该 owner 的未就绪或恢复合同处理，不能直接把旧结果当作当前结果返回。pending 不使用这个替换：它需要固定 through 下的当前成员，读取方案见[成员索引设计](appserver/timeline-member-index.md#5-读取)。
