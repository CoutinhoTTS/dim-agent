pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Enter toggle" => "Enter 切り替え",
        "Ctrl+R reload" => "Ctrl+R 再読み込み",
        "Enter toggle · Ctrl+R reload · Esc close" => {
            "Enter 切り替え · Ctrl+R 再読み込み · Esc 閉じる"
        }
        "Loading..." => "読み込み中…",
        "No entries." => "項目がありません。",
        "No matching entry." => "一致する項目がありません。",
        "Issues" => "問題",
        "enabled" => "有効",
        "shadowed by {name}" => "{name} が優先",
        "Reloaded." => "再読み込みしました。",
        "Enabled {name}." => "{name} を有効にしました。",
        "Disabled {name}." => "{name} を無効にしました。",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} を再読み込みしました：{entries} 件、{disabled} 件が無効。"
        }
        _ => return None,
    })
}
