pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "正在載入…",
        "No entries." => "沒有項目。",
        "No matching entry." => "沒有符合的項目。",
        "Issues" => "問題",
        "enabled" => "已啟用",
        "shadowed by {name}" => "被 {name} 覆蓋",
        "Reloaded." => "已重新載入。",
        "Enabled {name}." => "已啟用 {name}。",
        "Disabled {name}." => "已停用 {name}。",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "已重新載入{area}：共 {entries} 項，{disabled} 項已停用。"
        }
        "Hooks" => "鉤子",
        "invalid" => "無效",
        "Saved." => "已儲存。",
        "No change." => "沒有變更。",
        "Your text is kept in {path}." => "你的內容保留在 {path}。",
        "The user's own document; always in effect." => "使用者自己的文件；始終生效。",
        "No .dimcode/hooks.json in this workspace." => "此工作區沒有 .dimcode/hooks.json。",
        "Owned by plugin {plugin}." => "由外掛 {plugin} 提供。",
        "The user document turns every hook off (disableAllHooks)." => {
            "使用者文件關閉了所有鉤子（disableAllHooks）。"
        }
        "Enter toggle" => "Enter 切換",
        "Ctrl+E edit" => "Ctrl+E 編輯",
        "Ctrl+N add" => "Ctrl+N 新增",
        "Ctrl+R reload" => "Ctrl+R 重新載入",
        "Ctrl+T run" => "Ctrl+T 執行",
        "Ctrl+X remove" => "Ctrl+X 刪除",
        "Tab inspect" => "Tab 檢視",
        "Remove {name}? Enter confirm · Esc cancel" => "刪除 {name}？Enter 確認 · Esc 取消",
        "↑↓ scroll · Esc back" => "↑↓ 捲動 · Esc 返回",
        "Added." => "已新增。",
        "Removed {name}." => "已刪除 {name}。",
        "Ran {name}." => "已執行 {name}。",
        "The user document changed since it was read; nothing was written. Your next edit opens the current document." => {
            "使用者文件在讀取後已被修改；這次沒有寫入。下次編輯會開啟目前文件。"
        }
        _ => return None,
    })
}
