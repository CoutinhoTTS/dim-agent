pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "正在加载…",
        "No entries." => "没有条目。",
        "No matching entry." => "没有匹配的条目。",
        "Issues" => "问题",
        "sources" => "来源",
        "skills" => "技能",
        "hooks" => "钩子",
        "scheduled jobs" => "定时任务",
        "models" => "模型",
        "command rules" => "命令规则",
        "enabled" => "已启用",
        "shadowed by {name}" => "被 {name} 覆盖",
        "Reloaded." => "已重新加载。",
        "Enabled {name}." => "已启用 {name}。",
        "Disabled {name}." => "已禁用 {name}。",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "已重新加载{area}：共 {entries} 项，{disabled} 项已禁用。"
        }
        "Reloaded {area}: {entries} entries." => "已重新加载{area}：共 {entries} 项。",
        "Hooks" => "钩子",
        "invalid" => "无效",
        "Saved." => "已保存。",
        "Saved {name}." => "已保存 {name}。",
        "No change." => "没有变化。",
        "Your text is kept in {path}." => "你的内容保留在 {path}。",
        "The user's own document; always in effect." => "用户自己的文档；始终生效。",
        "There is no hooks file there." => "那里没有 hooks 文件。",
        "Owned by plugin {plugin}." => "由插件 {plugin} 提供。",
        "The user document turns every hook off (disableAllHooks)." => {
            "用户文档关闭了所有钩子（disableAllHooks）。"
        }
        "Enter toggle" => "Enter 切换",
        "Ctrl+E edit" => "Ctrl+E 编辑",
        "Ctrl+N add" => "Ctrl+N 新建",
        "Ctrl+R reload" => "Ctrl+R 重新加载",
        "Ctrl+T run" => "Ctrl+T 运行",
        "Ctrl+X remove" => "Ctrl+X 删除",
        "Tab inspect" => "Tab 查看",
        "Remove {name}? Enter confirm · Esc cancel" => "删除 {name}？Enter 确认 · Esc 取消",
        "↑↓ scroll · Esc back" => "↑↓ 滚动 · Esc 返回",
        "Added." => "已新建。",
        "Removed {name}." => "已删除 {name}。",
        "unavailable" => "不可用",
        "Reset {name}? Enter confirm · Esc cancel" => "重置 {name}？Enter 确认 · Esc 取消",
        "Reset {name} to its catalog entry." => "已将 {name} 重置为目录条目。",
        "Ran {name}." => "已运行 {name}。",
        "The user document changed since it was read; nothing was written." => {
            "用户文档在读取后已被修改；本次没有写入。"
        }
        "The job changed since it was read; nothing was written." => {
            "任务在读取后已被修改；本次没有写入。"
        }
        "The jobs changed since they were read; nothing was written." => {
            "任务在读取后已被修改；本次没有写入。"
        }
        "Your next edit opens the current document." => "下次编辑会打开当前文档。",
        "Your next add reopens your text." => "下次新建会重新打开你的文本。",
        "Scheduler" => "调度器",
        "jobs" => "任务",
        "Added {name}." => "已新建 {name}。",
        "Run {name}: {outcome}." => "已运行 {name}：{outcome}。",
        "Recent runs of {name}" => "{name} 的最近运行",
        "No run yet." => "尚未运行。",
        "No next run." => "没有下次运行。",
        "Next run {instant}." => "下次运行 {instant}。",
        "Last run {outcome} at {instant}." => "上次运行 {outcome}，于 {instant}。",
        "every {minutes} minutes" => "每 {minutes} 分钟",
        "at {instant}" => "于 {instant}",
        "cron {expression}" => "cron {expression}",
        "cron {expression} in {zone}" => "cron {expression}（{zone}）",
        "The job changed since it was read; nothing was removed." => {
            "任务在读取后已被修改；本次没有删除。"
        }
        "The Scheduler has no area document." => "调度器没有领域文档。",
        "No scheduled job has that id." => "没有该 id 的定时任务。",
        "This daemon cannot manage {area}." => "无法管理{area}。",
        "This daemon lists {area} but cannot toggle them." => "可以列出{area}，但无法切换它们。",
        "This daemon lists {area} but cannot edit them." => "可以列出{area}，但无法编辑它们。",
        "This daemon lists {area} but cannot reload them." => {
            "可以列出{area}，但无法重新加载它们。"
        }
        "This daemon lists {area} but cannot add them." => "可以列出{area}，但无法添加它们。",
        "This daemon lists {area} but cannot remove them." => "可以列出{area}，但无法删除它们。",
        "This daemon lists {area} but cannot run them." => "可以列出{area}，但无法运行它们。",
        "This daemon lists {area} but cannot inspect them." => {
            "可以列出{area}，但无法查看它们的详情或历史。"
        }
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "文档删除了 {field}，所有者无法清除它；本次没有写入。"
        }
        "The document changes no field; nothing was written." => {
            "文档没有改动任何字段；本次没有写入。"
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "此工作区的钩子设置已在读取后发生变化；本次没有写入。"
        }
        "Models" => "模型",
        "catalog" => "目录",
        "overrides" => "覆盖",
        "added" => "新增",
        "Models · {name}" => "模型 · {name}",
        "undeclared" => "未声明",
        "Context window {window} · max output {output}" => {
            "上下文窗口 {window} · 最大输出 {output}"
        }
        "Endpoint {endpoint}" => "端点 {endpoint}",
        "Endpoint {endpoint} · override {value}" => "端点 {endpoint} · 覆盖为 {value}",
        "The provider reports it unavailable." => "提供方报告它不可用。",
        "Limits undeclared; edit the model to declare them." => "上限未声明；编辑该模型以声明。",
        "Sign in to the account to use its models." => "登录账号后即可使用其模型。",
        "The signed-in account changed; reload to list its models." => {
            "登录的账号已变更；重新加载以列出其模型。"
        }
        "The provider has no catalog yet." => "此提供方还没有目录。",
        "The provider listed no models." => "提供方没有列出任何模型。",
        "Listed {count} models." => "已列出 {count} 个模型。",
        "Refreshed the models." => "模型已刷新。",
        "Could not list models: {error}. Showing the stored list." => {
            "无法列出模型：{error}。显示已存储的列表。"
        }
        "The signed-in account changed; the refresh cleared preferences, overrides and disables." => {
            "登录的账号已变更；刷新已清除偏好、覆盖和禁用项。"
        }
        "The models changed since they were read; nothing was removed." => {
            "模型在读取后已变更；本次没有移除任何东西。"
        }
        "The models changed since they were read; nothing was written." => {
            "模型在读取后已变更；本次没有写入。"
        }
        "No provider has that id." => "没有该 id 的提供方。",
        "{name} is unavailable ({reason}); its enablement is kept as it is." => {
            "{name} 不可用（{reason}）；其启用状态保持不变。"
        }
        "No model has that id." => "没有该 id 的模型。",
        "This model cannot be removed." => "此模型无法移除。",
        "A Provider's models have no area document." => "提供方的模型没有领域文档。",
        "Models have no run operation." => "模型没有运行操作。",
        "The document is not an object; nothing was written." => "文档不是对象；本次没有写入。",
        "The document changes no editable field; nothing was written." => {
            "文档没有修改任何可编辑字段；本次没有写入。"
        }
        "The model document names no model." => "模型文档没有指明模型。",
        "The provider has no model catalog to write." => "此提供方没有可写入的模型目录。",
        "ID {id}" => "ID {id}",
        "Source {source}" => "来源 {source}",
        "Status {status}" => "状态 {status}",
        "Tools {value}" => "工具 {value}",
        "Vision {value}" => "视觉 {value}",
        "Reasoning {value}" => "推理 {value}",
        "Modalities {list}" => "模态 {list}",
        "Description {text}" => "描述 {text}",
        "Release date {date}" => "发布日期 {date}",
        "Pricing {value}" => "价格 {value}",
        "Rate limit {value}" => "速率限制 {value}",
        "Matched provider {id}" => "匹配提供方 {id}",
        "Matched model {id}" => "匹配模型 {id}",
        "Match strategy {strategy}" => "匹配策略 {strategy}",
        "Capability overrides {fields}" => "能力覆盖字段 {fields}",
        "yes" => "是",
        "no" => "否",
        "not supported" => "不支持",
        "supported" => "支持",
        "Command rules" => "命令规则",
        "user" => "用户",
        "Matches only this command." => "仅匹配此命令。",
        "Matches commands whose first words are these." => "匹配以此开头的命令。",
        "Allows a simple command that would otherwise ask." => "允许本会询问的简单命令。",
        "Denies the command under every preset." => "在任何预设下都拒绝该命令。",
        "Applies in every Session, in every workspace." => "在所有会话、所有工作区中生效。",
        "Rule added: {rule}." => "已添加规则：{rule}。",
        "Rule already present: {rule}." => "规则已存在：{rule}。",
        "Rule removed: {rule}." => "已删除规则：{rule}。",
        "That command rule is no longer listed; nothing was removed." => {
            "该命令规则已不在列表中；未删除任何内容。"
        }
        "Command rules changed since they were read; nothing was removed." => {
            "命令规则在读取后已被修改；未删除任何内容。"
        }
        "A command rule is DECISION MATCH COMMAND, such as deny prefix git push." => {
            "命令规则写作 DECISION MATCH COMMAND，例如 deny prefix git push。"
        }
        "Command: {command}" => "命令：{command}",
        "Use the line prompt or dimcode permissions." => {
            "请在行内提示符或 dimcode permissions 中使用。"
        }
        _ => return None,
    })
}
