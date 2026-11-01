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
        "MCP" => "MCP",
        "Ctrl+T test" => "Ctrl+T testen",
        "sign-in required" => "Anmeldung erforderlich",
        "signed in" => "angemeldet",
        "sign-in expired" => "Anmeldung abgelaufen",
        "missing scopes: {scopes}" => "fehlende Bereiche: {scopes}",
        "Executable: {command}" => "Ausführbar: {command}",
        "Stored arguments (hidden): {count}" => "Gespeicherte Argumente (verborgen): {count}",
        "Working directory: {cwd}" => "Arbeitsverzeichnis: {cwd}",
        "Environment names: {names}" => "Umgebungsnamen: {names}",
        "Endpoint: {endpoint}" => "Endpunkt: {endpoint}",
        "Header names: {names}" => "Header-Namen: {names}",
        "Bearer token: {state}" => "Bearer-Token: {state}",
        "OAuth: configured (client ID {id})" => "OAuth: konfiguriert (Client-ID {id})",
        "OAuth: configured" => "OAuth: konfiguriert",
        "OAuth: not configured" => "OAuth: nicht konfiguriert",
        "Metadata URL: {state}" => "Metadaten-URL: {state}",
        "configured" => "konfiguriert",
        "Owned by a plugin; change it through Plugins." => {
            "Von einem Plugin bereitgestellt; ändern Sie es über Plugins."
        }
        "MCP entry {id}" => "MCP-Eintrag {id}",
        "Connection {number} · {status}" => "Verbindung {number} · {status}",
        "Tools have not been discovered." => "Tools wurden noch nicht ermittelt.",
        "No live session connections." => "Keine aktiven Sitzungsverbindungen.",
        "Tools of {id}" => "Tools von {id}",
        "The server exposes no tools." => "Der Server stellt keine Tools bereit.",
        "The add document needs an id." => "Das Hinzufügen-Dokument braucht eine id.",
        "The add document needs a source of user or project." => {
            "Die source des Hinzufügen-Dokuments ist user oder project."
        }
        "The add document needs a configuration object." => {
            "Das Hinzufügen-Dokument braucht ein configuration-Objekt."
        }
        "{id} is already in mcp list; remove it first or choose another id." => {
            "{id} steht bereits in mcp list; entfernen Sie es zuerst oder wählen Sie eine andere id."
        }
        "This entry's settings changed since they were read; nothing was written." => {
            "Die Einstellungen dieses Eintrags haben sich seit dem Lesen geändert; nichts wurde geschrieben."
        }
        "The MCP entry changed since it was read; nothing was written. Your next edit opens the current entry." => {
            "Der MCP-Eintrag hat sich seit dem Lesen geändert; nichts wurde geschrieben. Die nächste Bearbeitung öffnet den aktuellen Eintrag."
        }
        "The MCP entries changed since they were read; nothing was written. Your next add opens your text again." => {
            "Die MCP-Einträge haben sich seit dem Lesen geändert; nichts wurde geschrieben. Das nächste Hinzufügen öffnet Ihren Text erneut."
        }
        "The MCP entry changed since it was read; nothing was removed." => {
            "Der MCP-Eintrag hat sich seit dem Lesen geändert; nichts wurde entfernt."
        }
        "MCP's documents belong to one entry; this save names none." => {
            "MCP-Dokumente gehören zu je einem Eintrag; dieses Speichern nennt keinen."
        }
        "No MCP server has that id." => "Kein MCP-Server hat diese id.",
        "This workspace has no project MCP source." => {
            "Dieser Arbeitsbereich hat keine Projekt-MCP-Quelle."
        }
        "Enabled {id} ({source})." => "{id} ({source}) aktiviert.",
        "Disabled {id} ({source})." => "{id} ({source}) deaktiviert.",
        "It runs this workspace's .mcp.json entry, including later edits to it, in active and new sessions." => {
            "Es führt den .mcp.json-Eintrag dieses Arbeitsbereichs aus, einschließlich späterer Änderungen, in aktiven und neuen Sitzungen."
        }
        "{id} is not in mcp list; withdrew any stored enablement of it in this workspace." => {
            "{id} steht nicht in mcp list; eine gespeicherte Aktivierung in diesem Arbeitsbereich wurde zurückgenommen."
        }
        "The MCP catalog could not be read ({error}); withdrew this workspace's enablement of {id}. A user entry with this ID is unchanged." => {
            "Der MCP-Katalog konnte nicht gelesen werden ({error}); die Aktivierung von {id} in diesem Arbeitsbereich wurde zurückgenommen. Ein Benutzereintrag mit dieser ID bleibt unverändert."
        }
        "MCP configuration: {error}" => "MCP-Konfiguration: {error}",
        "The server rejected the stored credential; sign in again with mcp login." => {
            "Der Server hat die gespeicherten Anmeldedaten abgelehnt; melden Sie sich mit mcp login erneut an."
        }
        "The server requires additional scopes: {scopes}." => {
            "Der Server verlangt zusätzliche Bereiche: {scopes}."
        }
        "The server requires authorization; run mcp login ID." => {
            "Der Server verlangt eine Autorisierung; führen Sie mcp login ID aus."
        }
        "The probe ended: {status}." => "Die Probe endete: {status}.",
        _ => return None,
    })
}
