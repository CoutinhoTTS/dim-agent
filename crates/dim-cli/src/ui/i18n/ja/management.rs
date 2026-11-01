pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "読み込み中…",
        "No entries." => "項目がありません。",
        "No matching entry." => "一致する項目がありません。",
        "Issues" => "問題",
        "sources" => "ソース",
        "skills" => "スキル",
        "enabled" => "有効",
        "shadowed by {name}" => "{name} が優先",
        "Reloaded." => "再読み込みしました。",
        "Enabled {name}." => "{name} を有効にしました。",
        "Disabled {name}." => "{name} を無効にしました。",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} を再読み込みしました：{entries} 件、{disabled} 件が無効。"
        }
        "Hooks" => "フック",
        "invalid" => "無効",
        "Saved." => "保存しました。",
        "Saved {name}." => "{name} を保存しました。",
        "No change." => "変更はありません。",
        "Your text is kept in {path}." => "入力した内容は {path} に残しています。",
        "The user's own document; always in effect." => {
            "ユーザー自身のドキュメント。常に有効です。"
        }
        "No .dimcode/hooks.json in this workspace." => {
            "このワークスペースに .dimcode/hooks.json はありません。"
        }
        "Owned by plugin {plugin}." => "プラグイン {plugin} が所有しています。",
        "The user document turns every hook off (disableAllHooks)." => {
            "ユーザードキュメントがすべてのフックを無効にしています（disableAllHooks）。"
        }
        "Enter toggle" => "Enter 切り替え",
        "Ctrl+E edit" => "Ctrl+E 編集",
        "Ctrl+N add" => "Ctrl+N 作成",
        "Ctrl+R reload" => "Ctrl+R 再読み込み",
        "Ctrl+T run" => "Ctrl+T 実行",
        "Ctrl+X remove" => "Ctrl+X 削除",
        "Tab inspect" => "Tab 詳細",
        "Remove {name}? Enter confirm · Esc cancel" => {
            "{name} を削除しますか？Enter で確定 · Esc でキャンセル"
        }
        "↑↓ scroll · Esc back" => "↑↓ スクロール · Esc で戻る",
        "Added." => "作成しました。",
        "Removed {name}." => "{name} を削除しました。",
        "Ran {name}." => "{name} を実行しました。",
        "The user document changed since it was read; nothing was written. Your next edit opens the current document." => {
            "ユーザードキュメントは読み取り後に変更されています。今回は何も書き込んでいません。次回の編集では現在のドキュメントを開きます。"
        }
        "Scheduler" => "スケジューラー",
        "Added {name}." => "{name} を作成しました。",
        "Run {name}: {outcome}." => "{name} を実行しました：{outcome}。",
        "Recent runs of {name}" => "{name} の最近の実行",
        "No run yet." => "まだ実行はありません。",
        "No next run." => "次回の実行はありません。",
        "Next run {instant}." => "次回の実行 {instant}。",
        "Last run {outcome} at {instant}." => "前回の実行 {outcome}（{instant}）。",
        "every {minutes} minutes" => "{minutes} 分ごと",
        "at {instant}" => "{instant} に",
        "cron {expression}" => "cron {expression}",
        "cron {expression} in {zone}" => "cron {expression}（{zone}）",
        "The job changed since it was read; nothing was written. Your next edit opens the current job." => {
            "ジョブは読み取り後に変更されています。今回は何も書き込んでいません。次回の編集では現在のジョブを開きます。"
        }
        "The jobs changed since they were read; nothing was written. Your next add opens your text again." => {
            "ジョブは読み取り後に変更されています。今回は何も書き込んでいません。次回の作成では入力した内容を開きます。"
        }
        "The job changed since it was read; nothing was removed." => {
            "ジョブは読み取り後に変更されています。今回は何も削除していません。"
        }
        "The Scheduler has no area document." => "スケジューラーには領域ドキュメントがありません。",
        "No scheduled job has that id." => "その id のスケジュールジョブはありません。",
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "ドキュメントから {field} が削除されましたが、所有者は消去できません。何も書き込みませんでした。"
        }
        "The document changes no field; nothing was written." => {
            "ドキュメントはどのフィールドも変更していません。何も書き込みませんでした。"
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "このワークスペースのフック設定は読み取り後に変更されています。今回は何も書き込んでいません。"
        }
        _ => return None,
    })
}
