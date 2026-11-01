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
        "Saved {name}." => "{name} gespeichert.",
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
        "The user document changed since it was read; nothing was written." => {
            "Das Benutzerdokument wurde seit dem Lesen geändert; es wurde nichts geschrieben."
        }
        "The job changed since it was read; nothing was written." => {
            "Der Job wurde seit dem Lesen geändert; es wurde nichts geschrieben."
        }
        "The jobs changed since they were read; nothing was written." => {
            "Die Jobs wurden seit dem Lesen geändert; es wurde nichts geschrieben."
        }
        "Your next edit opens the current document." => {
            "Deine nächste Bearbeitung öffnet das aktuelle Dokument."
        }
        "Your next add reopens your text." => "Dein nächstes Hinzufügen öffnet deinen Text erneut.",
        "Scheduler" => "Scheduler",
        "jobs" => "Jobs",
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
        "The job changed since it was read; nothing was removed." => {
            "Der Job wurde seit dem Lesen geändert; es wurde nichts entfernt."
        }
        "The Scheduler has no area document." => "Der Scheduler hat kein Bereichsdokument.",
        "No scheduled job has that id." => "Kein geplanter Job hat diese ID.",
        "This daemon cannot manage {area}." => "Kann {area} nicht verwalten.",
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "Das Dokument hat {field} entfernt, was der Eigentümer nicht leeren kann; es wurde nichts geschrieben."
        }
        "The document changes no field; nothing was written." => {
            "Das Dokument ändert kein Feld; es wurde nichts geschrieben."
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "Die Hook-Einstellungen dieses Arbeitsbereichs wurden seit dem Lesen geändert; es wurde nichts geschrieben."
        }
        "Workflow" => "Workflow",
        "Saved {name}. It stays disabled until you enable it." => {
            "{name} gespeichert. Es bleibt deaktiviert, bis du es aktivierst."
        }
        "The definitions changed since they were read; nothing was written." => {
            "Die Definitionen wurden seit dem Lesen geändert; es wurde nichts geschrieben."
        }
        "A definition's name is its key; add it to save it under another name." => {
            "Der Name einer Definition ist ihr Schlüssel; füge sie hinzu, um sie unter einem anderen Namen zu speichern."
        }
        "No project definition is named {name}." => "Keine Projektdefinition heißt {name}.",
        "No workflow definition is named {name}." => "Keine Workflow-Definition heißt {name}.",
        "Saved in the user workflow source; always enabled." => {
            "In der Workflow-Quelle des Benutzers gespeichert; immer aktiviert."
        }
        "Starts only after you enable it." => "Startet erst, nachdem du es aktivierst.",
        "Defined by package {package}." => "Definiert durch Paket {package}.",
        "{name} — {description} (required)" => "{name} — {description} (erforderlich)",
        "{name} — {description}" => "{name} — {description}",
        "{name} — required" => "{name} — erforderlich",
        "{name} — default: {default}" => "{name} — Standard: {default}",
        "and {count} more" => "und {count} weitere",
        "Recent runs" => "Letzte Läufe",
        "A workflow definition is saved as an object with a source and a document." => {
            "Eine Workflow-Definition wird als Objekt mit einer Quelle und einem Dokument gespeichert."
        }
        "A definition is saved to the user source or the project source." => {
            "Eine Definition wird in der Benutzerquelle oder der Projektquelle gespeichert."
        }
        "Started {name}: {run_id}." => "{name} gestartet: {run_id}.",
        "Parameters are a JSON object of strings." => {
            "Parameter sind ein JSON-Objekt aus Zeichenketten."
        }
        _ => return None,
    })
}
