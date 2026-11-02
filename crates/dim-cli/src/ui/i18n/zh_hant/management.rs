pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "正在載入…",
        "No entries." => "沒有項目。",
        "No matching entry." => "沒有符合的項目。",
        "Issues" => "問題",
        "sources" => "來源",
        "skills" => "技能",
        "hooks" => "鉤子",
        "scheduled jobs" => "排程工作",
        "models" => "模型",
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
        "Saved {name}." => "已儲存 {name}。",
        "No change." => "沒有變更。",
        "Your text is kept in {path}." => "你的內容保留在 {path}。",
        "The user's own document; always in effect." => "使用者自己的文件；始終生效。",
        "There is no hooks file there." => "那裡沒有 hooks 檔案。",
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
        "unavailable" => "無法使用",
        "Reset {name}? Enter confirm · Esc cancel" => "重設 {name}？Enter 確認 · Esc 取消",
        "Reset {name} to its catalog entry." => "已將 {name} 重設為目錄項目。",
        "Ran {name}." => "已執行 {name}。",
        "The user document changed since it was read; nothing was written." => {
            "使用者文件在讀取後已被修改；這次沒有寫入。"
        }
        "The job changed since it was read; nothing was written." => {
            "工作在讀取後已被修改；這次沒有寫入。"
        }
        "The jobs changed since they were read; nothing was written." => {
            "工作在讀取後已被修改；這次沒有寫入。"
        }
        "Your next edit opens the current document." => "下次編輯會開啟目前文件。",
        "Your next add reopens your text." => "下次新增會重新開啟你的文字。",
        "Scheduler" => "排程器",
        "jobs" => "工作",
        "Added {name}." => "已新增 {name}。",
        "Run {name}: {outcome}." => "已執行 {name}：{outcome}。",
        "Recent runs of {name}" => "{name} 的最近執行",
        "No run yet." => "尚未執行。",
        "No next run." => "沒有下次執行。",
        "Next run {instant}." => "下次執行 {instant}。",
        "Last run {outcome} at {instant}." => "上次執行 {outcome}，於 {instant}。",
        "every {minutes} minutes" => "每 {minutes} 分鐘",
        "at {instant}" => "於 {instant}",
        "cron {expression}" => "cron {expression}",
        "cron {expression} in {zone}" => "cron {expression}（{zone}）",
        "The job changed since it was read; nothing was removed." => {
            "工作在讀取後已被修改；這次沒有刪除。"
        }
        "The Scheduler has no area document." => "排程器沒有領域文件。",
        "No scheduled job has that id." => "沒有該 id 的排程工作。",
        "This daemon cannot manage {area}." => "無法管理{area}。",
        "This daemon lists {area} but cannot toggle them." => "可以列出{area}，但無法切換它們。",
        "This daemon lists {area} but cannot edit them." => "可以列出{area}，但無法編輯它們。",
        "This daemon lists {area} but cannot reload them." => {
            "可以列出{area}，但無法重新載入它們。"
        }
        "This daemon lists {area} but cannot add them." => "可以列出{area}，但無法新增它們。",
        "This daemon lists {area} but cannot remove them." => "可以列出{area}，但無法刪除它們。",
        "This daemon lists {area} but cannot run them." => "可以列出{area}，但無法執行它們。",
        "This daemon lists {area} but cannot inspect them." => {
            "可以列出{area}，但無法檢視它們的詳細資料或歷史。"
        }
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "文件刪除了 {field}，擁有者無法清除它；本次沒有寫入。"
        }
        "The document changes no field; nothing was written." => {
            "文件沒有變更任何欄位；本次沒有寫入。"
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "此工作區的鉤子設定已在讀取後變更；這次沒有寫入。"
        }
        "Models" => "模型",
        "catalog" => "目錄",
        "overrides" => "覆寫",
        "added" => "新增",
        "Models · {name}" => "模型 · {name}",
        "undeclared" => "未宣告",
        "Context window {window} · max output {output}" => {
            "上下文視窗 {window} · 最大輸出 {output}"
        }
        "Endpoint {endpoint}" => "端點 {endpoint}",
        "Endpoint {endpoint} · override {value}" => "端點 {endpoint} · 覆寫為 {value}",
        "The provider reports it unavailable." => "提供方回報它無法使用。",
        "Limits undeclared; edit the model to declare them." => "上限未宣告；編輯該模型以宣告。",
        "Sign in to the account to use its models." => "登入帳號後即可使用其模型。",
        "The signed-in account changed; reload to list its models." => {
            "登入的帳號已變更；重新載入以列出其模型。"
        }
        "The provider has no catalog yet." => "此提供方還沒有目錄。",
        "The provider listed no models." => "提供方沒有列出任何模型。",
        "Listed {count} models." => "已列出 {count} 個模型。",
        "Refreshed the models." => "模型已重新整理。",
        "Could not list models: {error}. Showing the stored list." => {
            "無法列出模型：{error}。顯示已儲存的清單。"
        }
        "The signed-in account changed; the refresh cleared preferences, overrides and disables." => {
            "登入的帳號已變更；重新整理已清除偏好、覆寫和停用項。"
        }
        "The models changed since they were read; nothing was removed." => {
            "模型在讀取後已變更；本次沒有移除任何東西。"
        }
        "The models changed since they were read; nothing was written." => {
            "模型在讀取後已變更；本次沒有寫入。"
        }
        "No provider has that id." => "沒有該 id 的提供方。",
        "{name} is unavailable ({reason}); its enablement is kept as it is." => {
            "{name} 無法使用（{reason}）；其啟用狀態保持不變。"
        }
        "No model has that id." => "沒有該 id 的模型。",
        "This model cannot be removed." => "此模型無法移除。",
        "A Provider's models have no area document." => "提供方的模型沒有領域文件。",
        "Models have no run operation." => "模型沒有執行操作。",
        "The document is not an object; nothing was written." => "文件不是物件；本次沒有寫入。",
        "The document changes no editable field; nothing was written." => {
            "文件沒有修改任何可編輯欄位；本次沒有寫入。"
        }
        "The model document names no model." => "模型文件沒有指明模型。",
        "The provider has no model catalog to write." => "此提供方沒有可寫入的模型目錄。",
        "ID {id}" => "ID {id}",
        "Source {source}" => "來源 {source}",
        "Status {status}" => "狀態 {status}",
        "Tools {value}" => "工具 {value}",
        "Vision {value}" => "視覺 {value}",
        "Reasoning {value}" => "推理 {value}",
        "Modalities {list}" => "模態 {list}",
        "Description {text}" => "說明 {text}",
        "Release date {date}" => "發佈日期 {date}",
        "Pricing {value}" => "價格 {value}",
        "Rate limit {value}" => "速率限制 {value}",
        "Matched provider {id}" => "符合提供方 {id}",
        "Matched model {id}" => "符合模型 {id}",
        "Match strategy {strategy}" => "符合策略 {strategy}",
        "Capability overrides {fields}" => "能力覆寫欄位 {fields}",
        "yes" => "是",
        "no" => "否",
        "not supported" => "不支援",
        "supported" => "支援",
        _ => return None,
    })
}
