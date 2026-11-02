pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "読み込み中…",
        "No entries." => "項目がありません。",
        "No matching entry." => "一致する項目がありません。",
        "Issues" => "問題",
        "sources" => "ソース",
        "skills" => "スキル",
        "hooks" => "フック",
        "scheduled jobs" => "スケジュール済みジョブ",
        "models" => "モデル",
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
        "There is no hooks file there." => "そこにフックファイルはありません。",
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
        "unavailable" => "利用不可",
        "Reset {name}? Enter confirm · Esc cancel" => {
            "{name} をリセットしますか？Enter で確定 · Esc でキャンセル"
        }
        "Reset {name} to its catalog entry." => "{name} をカタログ項目にリセットしました。",
        "Ran {name}." => "{name} を実行しました。",
        "The user document changed since it was read; nothing was written." => {
            "ユーザードキュメントは読み取り後に変更されています。今回は何も書き込んでいません。"
        }
        "The job changed since it was read; nothing was written." => {
            "ジョブは読み取り後に変更されています。今回は何も書き込んでいません。"
        }
        "The jobs changed since they were read; nothing was written." => {
            "ジョブは読み取り後に変更されています。今回は何も書き込んでいません。"
        }
        "Your next edit opens the current document." => {
            "次回の編集では現在のドキュメントを開きます。"
        }
        "Your next add reopens your text." => "次回の作成では入力した内容を開きます。",
        "Scheduler" => "スケジューラー",
        "jobs" => "ジョブ",
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
        "The job changed since it was read; nothing was removed." => {
            "ジョブは読み取り後に変更されています。今回は何も削除していません。"
        }
        "The Scheduler has no area document." => "スケジューラーには領域ドキュメントがありません。",
        "No scheduled job has that id." => "その id のスケジュールジョブはありません。",
        "This daemon cannot manage {area}." => "{area} を管理できません。",
        "This daemon lists {area} but cannot toggle them." => {
            "{area} の一覧は表示できますが、切り替えはできません。"
        }
        "This daemon lists {area} but cannot edit them." => {
            "{area} の一覧は表示できますが、編集はできません。"
        }
        "This daemon lists {area} but cannot reload them." => {
            "{area} の一覧は表示できますが、再読み込みはできません。"
        }
        "This daemon lists {area} but cannot add them." => {
            "{area} の一覧は表示できますが、追加はできません。"
        }
        "This daemon lists {area} but cannot remove them." => {
            "{area} の一覧は表示できますが、削除はできません。"
        }
        "This daemon lists {area} but cannot run them." => {
            "{area} の一覧は表示できますが、実行はできません。"
        }
        "This daemon lists {area} but cannot inspect them." => {
            "{area} の一覧は表示できますが、詳細や履歴は見られません。"
        }
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "ドキュメントから {field} が削除されましたが、所有者は消去できません。何も書き込みませんでした。"
        }
        "The document changes no field; nothing was written." => {
            "ドキュメントはどのフィールドも変更していません。何も書き込みませんでした。"
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "このワークスペースのフック設定は読み取り後に変更されています。今回は何も書き込んでいません。"
        }
        "Models" => "モデル",
        "catalog" => "カタログ",
        "overrides" => "オーバーライド",
        "added" => "追加",
        "Models · {name}" => "モデル · {name}",
        "undeclared" => "未宣言",
        "Context window {window} · max output {output}" => {
            "コンテキストウィンドウ {window} · 最大出力 {output}"
        }
        "Endpoint {endpoint}" => "エンドポイント {endpoint}",
        "Endpoint {endpoint} · override {value}" => {
            "エンドポイント {endpoint} · オーバーライド {value}"
        }
        "The provider reports it unavailable." => "プロバイダーは利用不可と報告しています。",
        "Limits undeclared; edit the model to declare them." => {
            "上限が未宣言です。モデルを編集して宣言してください。"
        }
        "Sign in to the account to use its models." => {
            "アカウントにサインインするとモデルを使えます。"
        }
        "The signed-in account changed; reload to list its models." => {
            "サインイン中のアカウントが変わりました。再読み込みでモデルを一覧します。"
        }
        "The provider has no catalog yet." => "このプロバイダーにはまだカタログがありません。",
        "The provider listed no models." => "プロバイダーはモデルを何も一覧しませんでした。",
        "Listed {count} models." => "{count} 個のモデルを一覧しました。",
        "Refreshed the models." => "モデルを更新しました。",
        "Could not list models: {error}. Showing the stored list." => {
            "モデルを一覧できませんでした：{error}。保存済みの一覧を表示します。"
        }
        "The signed-in account changed; the refresh cleared preferences, overrides and disables." => {
            "サインイン中のアカウントが変わりました。更新により設定、オーバーライド、無効化が消去されました。"
        }
        "The models changed since they were read; nothing was removed." => {
            "モデルは読み取り後に変更されています。今回は何も削除していません。"
        }
        "The models changed since they were read; nothing was written." => {
            "モデルは読み取り後に変更されています。今回は何も書き込んでいません。"
        }
        "No provider has that id." => "その id のプロバイダーはありません。",
        "{name} is unavailable ({reason}); its enablement is kept as it is." => {
            "{name} は利用できません（{reason}）。有効状態はそのまま維持されます。"
        }
        "No model has that id." => "その id のモデルはありません。",
        "This model cannot be removed." => "このモデルは削除できません。",
        "A Provider's models have no area document." => {
            "プロバイダーのモデルには領域ドキュメントがありません。"
        }
        "Models have no run operation." => "モデルに実行操作はありません。",
        "The document is not an object; nothing was written." => {
            "ドキュメントがオブジェクトではありません。今回は何も書き込んでいません。"
        }
        "The document changes no editable field; nothing was written." => {
            "ドキュメントは編集可能なフィールドを変更していません。今回は何も書き込んでいません。"
        }
        "The model document names no model." => "モデルドキュメントがモデルを指定していません。",
        "The provider has no model catalog to write." => {
            "このプロバイダーには書き込めるモデルカタログがありません。"
        }
        "ID {id}" => "ID {id}",
        "Source {source}" => "ソース {source}",
        "Status {status}" => "ステータス {status}",
        "Tools {value}" => "ツール {value}",
        "Vision {value}" => "ビジョン {value}",
        "Reasoning {value}" => "推論 {value}",
        "Modalities {list}" => "モダリティ {list}",
        "Description {text}" => "説明 {text}",
        "Release date {date}" => "リリース日 {date}",
        "Pricing {value}" => "価格 {value}",
        "Rate limit {value}" => "レート制限 {value}",
        "Matched provider {id}" => "一致プロバイダー {id}",
        "Matched model {id}" => "一致モデル {id}",
        "Match strategy {strategy}" => "一致戦略 {strategy}",
        "Capability overrides {fields}" => "ケイパビリティの上書き {fields}",
        "yes" => "はい",
        "no" => "いいえ",
        "not supported" => "非対応",
        "supported" => "対応",
        _ => return None,
    })
}
