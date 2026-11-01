pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Enter toggle" => "Enter 切换",
        "Ctrl+R reload" => "Ctrl+R 重新加载",
        "Loading..." => "正在加载…",
        "No entries." => "没有条目。",
        "No matching entry." => "没有匹配的条目。",
        "Issues" => "问题",
        "enabled" => "已启用",
        "shadowed by {name}" => "被 {name} 覆盖",
        "Reloaded." => "已重新加载。",
        "Enabled {name}." => "已启用 {name}。",
        "Disabled {name}." => "已禁用 {name}。",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "已重新加载{area}：共 {entries} 项，{disabled} 项已禁用。"
        }
        _ => return None,
    })
}
