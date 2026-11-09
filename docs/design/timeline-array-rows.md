# Timeline：数组元素查询

Status: Draft.
Scope: Timeline 查询合同的元素行扩展。
Issue: [#1320](https://github.com/arcships/dim/issues/1320)。
基础合同：[属性查询](timeline-property-query.md)、[插件查询入口](plugin-contract.md#54-the-event-query)。

## 1. 决定

沿用 query，不新增读取服务。默认输入仍是一条事件；调用方可以指定一个数组字段，使数组中的每个元素成为一行。本文定义提案，不表示接口已存在。

这与现有 Fields 的 `[]` 投影不同：原投影保留数组形状；元素行可以独立筛选、分组、排序和分页。

## 2. 请求与字段

在原 query 对象增加可选 rows：

```json
{
  "rows": {"kind": "elements", "field": "data.changes"},
  "filter": null,
  "select": {"kind": "fields", "fields": [
    "position", "element.index", "element.value"
  ]},
  "group_by": [],
  "pick": null,
  "order_by": []
}
```

| 字段 | 合同 |
| --- | --- |
| rows 省略 | 原事件行，结果和游标语义不变。 |
| rows.kind | 首版只增加 elements；不增加任意数据源或 SQL。 |
| rows.field | 一个公开事件数组字段；不能包含 `[]` 或再展开第二层数组。 |
| element.index | 原数组的零基下标；筛选不重编号。 |
| element.value | 该元素的原始 JSON 值，可读取其子路径。 |
| 原有事件属性 | 仍指包含该元素的事件。每个元素共享同一事件位置。 |

缺失、null、空数组产生零行；非数组值返回查询类型错误，不能当作空集合。数组内的 null 仍是一行。请求完整 Events 与 elements 不能组合，不能把元素伪装成事件。

Fields、Count、Exists 和标量分组可用于元素行。Characters 保留字符串字符单位。组内 First/Last 按分支段、事件序号、元素下标选择；order_by 只排列最终结果。事件行与元素行的游标不能混用。

## 3. 条件和关联

根 Field 可以读取事件属性及 element.index/value。Element 操作数仍只用于原 Any 条件，不增加另一种读取同一根元素的写法。

Exists 的内层仍是同一固定范围中的事件，不隐式展开另一个数组。Outer 指外层结果行，因此可以引用外层事件位置和 element.index/value。内层 Any 的 Element 只指它正在判断的数组元素，不指外层元素。需要引用外层元素时必须使用 Outer。

元素行的下标不改变原事件位置，不能用筛选后相邻行代替原数组下标。首版不增加 Any 内的下标操作数、嵌套 Exists、嵌套 Any、任意函数、CASE 或用户 SQL。

Prompt 正常恢复按已保存的事件 ID/下标读取有效 Set，不要求 query 猜测 Set.entry.source 与 Revoke.source 的对应关系。历史初始化才解释两种操作。当前条目的维护与有效性见 [Prompt 设计](prompt-state-read.md)。

## 4. 固定范围和分页

先固定分支链和 through，再在该范围展开、筛选、关联、分组，最后分页。排序的末尾唯一键包含元素下标。续页保留原 through，不能以页游标缩小关联范围。

同一事件的元素可以跨页；调用方不能把元素页当成原子事件批次。取消释放本次查询的数组、临时索引和正文引用。

## 5. 执行目标

以下 SQL 只说明元素行语义，events 是既有逻辑事件范围：

```sql
SELECT e.position, c.key AS element_index, c.value AS element_value
FROM events e, json_each(e.changes) c;
```

存储先用事件类型、范围、身份或元素索引缩小输入，再读取必要数组。按事件 ID 和下标点选时，直接取得元素，不先展开整数组。大数组元素定位与内容编码由[下一格式提案](appserver/native-storage-format.md#提案状态读取索引与数组元素)定义，不为返回几十个有效条目展开所有历史 changes。元素索引保存身份与内容引用，不复制另一份权威正文。

Memory 与 SQLite 返回相同元素身份、缺失值和顺序。合并物理行先恢复逻辑事件位置。引用内容只在所选属性要求时加载。数组行能力只解决表达问题，不自动提供有效条目索引或业务生命周期解释。

## 6. 验收

- 同事件内 Set/Revoke 的先后次序正确；同键重复操作、null 元素、缺失和非数组分别验证。
- 选择、关联、分组和续页在元素跨页、祖先截止、publishing tail 下结果一致。
- 旧事件行请求及其游标不变；新查询在旧服务端明确报不支持，不改成全历史读取。
- 历史操作增长十倍、当前有效条目不变时，分别检查元素选择计划和返回量；不得只检查最后输出条数。

## 7. Rust 与跨语言绑定提案

`QuerySpec` 增加可选 `rows: QueryRows`；新增 `QueryRows::Elements { field }`。字段路径校验允许元素行中的 element.index/value；Any 的 Element 合同不变。Outer 的 element.index/value 总是外层输出元素，不新增操作数。

rows=None 时省略序列化字段，沿用旧 query binding；不能让一个默认 rows:null 使已有事件游标无故失效。新元素游标明确绑定 rows 描述及下标。

QueryRow 的返回形状不增加另一套结果类型；调用方显式选择位置和下标。排序/续页内部补齐元素身份，即使调用方没有选择它们。查询类型修改时同步 Rust、Memory、SQLite、服务声明和生成绑定，不新增语言专用接口。

删减：只有单层元素行和元素身份；不增加关系 DSL、查询会话、Reader、计算字段或业务路径别名。
