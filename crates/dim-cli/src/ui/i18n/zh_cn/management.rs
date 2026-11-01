pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "正在加载…",
        "No entries." => "没有条目。",
        "No matching entry." => "没有匹配的条目。",
        "Issues" => "问题",
        "sources" => "来源",
        "skills" => "技能",
        "enabled" => "已启用",
        "shadowed by {name}" => "被 {name} 覆盖",
        "Reloaded." => "已重新加载。",
        "Enabled {name}." => "已启用 {name}。",
        "Disabled {name}." => "已禁用 {name}。",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "已重新加载{area}：共 {entries} 项，{disabled} 项已禁用。"
        }
        "Hooks" => "钩子",
        "invalid" => "无效",
        "Saved." => "已保存。",
        "Saved {name}." => "已保存 {name}。",
        "No change." => "没有变化。",
        "Your text is kept in {path}." => "你的内容保留在 {path}。",
        "The user's own document; always in effect." => "用户自己的文档；始终生效。",
        "No .dimcode/hooks.json in this workspace." => "此工作区没有 .dimcode/hooks.json。",
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
        "A scheduled job's run takes no input document." => "定时任务的运行不接受输入文档。",
        "This daemon cannot manage {area}." => "无法管理{area}。",
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "文档删除了 {field}，所有者无法清除它；本次没有写入。"
        }
        "The document changes no field; nothing was written." => {
            "文档没有改动任何字段；本次没有写入。"
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "此工作区的钩子设置已在读取后发生变化；本次没有写入。"
        }
        "Workflow" => "工作流",
        "Saved {name}. It stays disabled until you enable it." => {
            "已保存 {name}。在你启用它之前，它保持禁用。"
        }
        "The definitions changed since they were read; nothing was written." => {
            "定义在读取后已被修改；本次没有写入。"
        }
        "A definition's name is its key; add it to save it under another name." => {
            "定义的名称就是它的键；要用另一个名称保存，请新建它。"
        }
        "No project definition is named {name}." => "没有名为 {name} 的项目定义。",
        "No workflow definition is named {name}." => "没有名为 {name} 的工作流定义。",
        "Saved in the user workflow source; always enabled." => {
            "保存在用户工作流来源中；始终启用。"
        }
        "Starts only after you enable it." => "只有在你启用它之后才会启动。",
        "Defined by package {package}." => "由包 {package} 定义。",
        "{name} — {description} (required)" => "{name} — {description}（必填）",
        "{name} — {description}" => "{name} — {description}",
        "{name} — required" => "{name} — 必填",
        "{name} — default: {default}" => "{name} — 默认：{default}",
        "and {count} more" => "另有 {count} 项",
        "Recent runs" => "最近的运行",
        "A workflow definition is saved as an object with a source and a document." => {
            "工作流定义以包含来源和文档的对象保存。"
        }
        "A definition is saved to the user source or the project source." => {
            "定义保存到用户来源或项目来源。"
        }
        "Started {name}: {run_id}." => "已启动 {name}：{run_id}。",
        "Parameters are a JSON object of strings." => "参数是字符串组成的 JSON 对象。",
        _ => return None,
    })
}
