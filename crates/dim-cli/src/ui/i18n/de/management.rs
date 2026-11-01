pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "Wird geladen…",
        "No entries." => "Keine Einträge.",
        "No matching entry." => "Kein passender Eintrag.",
        "Issues" => "Probleme",
        "sources" => "Quellen",
        "skills" => "Skills",
        "enabled" => "aktiviert",
        "shadowed by {name}" => "von {name} verdeckt",
        "Reloaded." => "Neu geladen.",
        "Enabled {name}." => "{name} aktiviert.",
        "Disabled {name}." => "{name} deaktiviert.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} neu geladen: {entries} Einträge, {disabled} deaktiviert."
        }
        "Hooks" => "Hooks",
        "invalid" => "ungültig",
        "Saved." => "Gespeichert.",
        "No change." => "Keine Änderung.",
        "Your text is kept in {path}." => "Dein Text bleibt in {path} erhalten.",
        "The user's own document; always in effect." => {
            "Das eigene Dokument des Benutzers; immer wirksam."
        }
        "No .dimcode/hooks.json in this workspace." => {
            "Keine .dimcode/hooks.json in diesem Arbeitsbereich."
        }
        "Owned by plugin {plugin}." => "Gehört zum Plugin {plugin}.",
        "The user document turns every hook off (disableAllHooks)." => {
            "Das Benutzerdokument schaltet alle Hooks ab (disableAllHooks)."
        }
        "Enter toggle" => "Enter umschalten",
        "Ctrl+E edit" => "Ctrl+E bearbeiten",
        "Ctrl+N add" => "Ctrl+N hinzufügen",
        "Ctrl+R reload" => "Ctrl+R neu laden",
        "Ctrl+T run" => "Ctrl+T ausführen",
        "Ctrl+X remove" => "Ctrl+X entfernen",
        "Tab inspect" => "Tab ansehen",
        "Remove {name}? Enter confirm · Esc cancel" => {
            "{name} entfernen? Enter bestätigen · Esc abbrechen"
        }
        "↑↓ scroll · Esc back" => "↑↓ scrollen · Esc zurück",
        "Added." => "Hinzugefügt.",
        "Removed {name}." => "{name} entfernt.",
        "Ran {name}." => "{name} ausgeführt.",
        "The user document changed since it was read; nothing was written. Your next edit opens the current document." => {
            "Das Benutzerdokument wurde seit dem Lesen geändert; es wurde nichts geschrieben. Deine nächste Bearbeitung öffnet das aktuelle Dokument."
        }
        "Scheduler" => "Scheduler",
        "Added {name}." => "{name} hinzugefügt.",
        "Run {name}: {outcome}." => "{name} ausgeführt: {outcome}.",
        "Recent runs of {name}" => "Letzte Läufe von {name}",
        "No run yet." => "Noch kein Lauf.",
        "No next run." => "Kein nächster Lauf.",
        "Next run {instant}." => "Nächster Lauf {instant}.",
        "Last run {outcome} at {instant}." => "Letzter Lauf {outcome} um {instant}.",
        "every {minutes} minutes" => "alle {minutes} Minuten",
        "at {instant}" => "um {instant}",
        "cron {expression}" => "cron {expression}",
        "cron {expression} in {zone}" => "cron {expression} in {zone}",
        "The job changed since it was read; nothing was written. Your next edit opens the current job." => {
            "Der Job wurde seit dem Lesen geändert; es wurde nichts geschrieben. Deine nächste Bearbeitung öffnet den aktuellen Job."
        }
        "The jobs changed since they were read; nothing was written. Your next add opens your text again." => {
            "Die Jobs wurden seit dem Lesen geändert; es wurde nichts geschrieben. Dein nächstes Hinzufügen öffnet deinen Text erneut."
        }
        "The job changed since it was read; nothing was removed." => {
            "Der Job wurde seit dem Lesen geändert; es wurde nichts entfernt."
        }
        "The Scheduler has no area document." => "Der Scheduler hat kein Bereichsdokument.",
        "No scheduled job has that id." => "Kein geplanter Job hat diese ID.",
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "Die Hook-Einstellungen dieses Arbeitsbereichs wurden seit dem Lesen geändert; es wurde nichts geschrieben."
        }
        "Command rules" => "Befehlsregeln",
        "user" => "Benutzer",
        "Matches only this command." => "Passt nur auf diesen Befehl.",
        "Matches commands whose first words are these." => {
            "Passt auf Befehle, die mit diesen Wörtern beginnen."
        }
        "Allows a simple command that would otherwise ask." => {
            "Erlaubt einen einfachen Befehl, der sonst nachfragen würde."
        }
        "Denies the command under every preset." => {
            "Lehnt den Befehl unter jeder Voreinstellung ab."
        }
        "Applies in every Session, in every workspace." => {
            "Gilt in jeder Sitzung und in jedem Arbeitsbereich."
        }
        "Rule added: {rule}." => "Regel hinzugefügt: {rule}.",
        "Rule already present: {rule}." => "Regel ist bereits vorhanden: {rule}.",
        "Rule removed: {rule}." => "Regel entfernt: {rule}.",
        "That command rule is no longer listed; nothing was removed." => {
            "Diese Befehlsregel ist nicht mehr aufgeführt; es wurde nichts entfernt."
        }
        "Command rules changed since they were read; nothing was removed." => {
            "Die Befehlsregeln wurden seit dem Lesen geändert; es wurde nichts entfernt."
        }
        "A command rule is DECISION MATCH COMMAND, such as deny prefix git push." => {
            "Eine Befehlsregel lautet DECISION MATCH COMMAND, zum Beispiel deny prefix git push."
        }
        _ => return None,
    })
}
