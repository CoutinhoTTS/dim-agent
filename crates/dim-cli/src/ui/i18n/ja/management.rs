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
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "このワークスペースのフック設定は読み取り後に変更されています。今回は何も書き込んでいません。"
        }
        "Workflow" => "ワークフロー",
        "Saved {name}." => "{name} を保存しました。",
        "Saved {name}. It stays disabled until you enable it." => {
            "{name} を保存しました。有効にするまで無効のままです。"
        }
        "The definitions changed since they were read; nothing was written." => {
            "定義は読み取り後に変更されました。何も書き込まれていません。"
        }
        "The definition changed since it was read; nothing was written. Your next edit opens the current definition." => {
            "定義は読み取り後に変更されました。何も書き込まれていません。次回の編集では現在の定義が開きます。"
        }
        "The definitions changed since they were read; nothing was written. Your next add opens your text again." => {
            "定義は読み取り後に変更されました。何も書き込まれていません。次回の追加ではあなたのテキストが再び開きます。"
        }
        "A definition's name is its key; add it to save it under another name." => {
            "定義の名前はそのキーです。別の名前で保存するには追加してください。"
        }
        "No project definition is named {name}." => "{name} というプロジェクト定義はありません。",
        "No workflow definition is named {name}." => "{name} というワークフロー定義はありません。",
        "Saved in the user workflow source; always enabled." => {
            "ユーザーのワークフローソースに保存されています。常に有効です。"
        }
        "Starts only after you enable it." => "有効にした後にのみ開始します。",
        "Defined by package {package}." => "パッケージ {package} によって定義されています。",
        "{name} — {description} (required)" => "{name} — {description}（必須）",
        "{name} — {description}" => "{name} — {description}",
        "{name} — required" => "{name} — 必須",
        "{name} — default: {default}" => "{name} — 既定: {default}",
        "and {count} more" => "ほか {count} 件",
        "Recent runs" => "最近の実行",
        "A workflow definition is saved as an object with a source and a document." => {
            "ワークフロー定義は、ソースとドキュメントを持つオブジェクトとして保存します。"
        }
        "A definition is saved to the user source or the project source." => {
            "定義はユーザーソースまたはプロジェクトソースに保存します。"
        }
        "Started {name}: {run_id}." => "{name} を開始しました: {run_id}。",
        "Parameters are a JSON object of strings." => {
            "パラメータは文字列の JSON オブジェクトです。"
        }
        _ => return None,
    })
}
