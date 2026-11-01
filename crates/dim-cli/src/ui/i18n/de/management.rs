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
        "unavailable" => "nicht verfügbar",
        "Reset {name}? Enter confirm · Esc cancel" => {
            "{name} zurücksetzen? Enter bestätigen · Esc abbrechen"
        }
        "Reset {name} to its catalog entry." => "{name} auf den Katalogeintrag zurückgesetzt.",
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
        "Models" => "Modelle",
        "catalog" => "Katalog",
        "overrides" => "Überschreibungen",
        "added" => "hinzugefügt",
        "Models · {name}" => "Modelle · {name}",
        "undeclared" => "nicht deklariert",
        "Context window {window} · max output {output}" => {
            "Kontextfenster {window} · maximale Ausgabe {output}"
        }
        "Endpoint {endpoint}" => "Endpunkt {endpoint}",
        "Endpoint {endpoint} · override {value}" => "Endpunkt {endpoint} · Überschreibung {value}",
        "The provider reports it unavailable." => "Der Anbieter meldet es als nicht verfügbar.",
        "Limits undeclared; edit the model to declare them." => {
            "Limits sind nicht deklariert; bearbeite das Modell, um sie zu deklarieren."
        }
        "Sign in to the account to use its models." => {
            "Melde dich beim Konto an, um seine Modelle zu nutzen."
        }
        "The signed-in account changed; reload to list its models." => {
            "Das angemeldete Konto hat sich geändert; lade neu, um seine Modelle aufzulisten."
        }
        "The provider has no catalog yet." => "Der Anbieter hat noch keinen Katalog.",
        "The provider listed no models." => "Der Anbieter hat keine Modelle aufgelistet.",
        "Listed {count} models." => "{count} Modelle aufgelistet.",
        "Refreshed the models." => "Modelle aktualisiert.",
        "Could not list models: {error}. Showing the stored list." => {
            "Modelle konnten nicht aufgelistet werden: {error}. Die gespeicherte Liste wird gezeigt."
        }
        "The signed-in account changed; the refresh cleared preferences, overrides and disables." => {
            "Das angemeldete Konto hat sich geändert; das Neuladen hat Einstellungen, Überschreibungen und Deaktivierungen gelöscht."
        }
        "The models changed since they were read; nothing was written. Your next edit opens the current list." => {
            "Die Modelle haben sich seit dem Lesen geändert; es wurde nichts geschrieben. Die nächste Bearbeitung öffnet die aktuelle Liste."
        }
        "The models changed since they were read; nothing was written. Your next add opens your text again." => {
            "Die Modelle haben sich seit dem Lesen geändert; es wurde nichts geschrieben. Die nächste Neueingabe öffnet deinen Text erneut."
        }
        "The models changed since they were read; nothing was removed." => {
            "Die Modelle haben sich seit dem Lesen geändert; es wurde nichts entfernt."
        }
        "The models changed since they were read; nothing was written." => {
            "Die Modelle haben sich seit dem Lesen geändert; es wurde nichts geschrieben."
        }
        "No provider has that id." => "Kein Anbieter hat diese id.",
        "No model has that id." => "Kein Modell hat diese id.",
        "This model cannot be removed." => "Dieses Modell kann nicht entfernt werden.",
        "A Provider's models have no area document." => {
            "Die Modelle eines Anbieters haben kein Bereichsdokument."
        }
        "Models have no run operation." => "Modelle haben keine Ausführungsoperation.",
        "The document is not an object; nothing was written." => {
            "Das Dokument ist kein Objekt; es wurde nichts geschrieben."
        }
        "The document changes no editable field; nothing was written." => {
            "Das Dokument ändert kein bearbeitbares Feld; es wurde nichts geschrieben."
        }
        "The model document names no model." => "Das Modelldokument nennt kein Modell.",
        "The provider has no model catalog to write." => {
            "Dieser Anbieter hat keinen Modellkatalog zum Schreiben."
        }
        "ID {id}" => "ID {id}",
        "Source {source}" => "Quelle {source}",
        "Status {status}" => "Status {status}",
        "Tools {value}" => "Werkzeuge {value}",
        "Vision {value}" => "Vision {value}",
        "Reasoning {value}" => "Reasoning {value}",
        "Modalities {list}" => "Modalitäten {list}",
        "Description {text}" => "Beschreibung {text}",
        "Release date {date}" => "Veröffentlichung {date}",
        "Pricing {value}" => "Preis {value}",
        "Rate limit {value}" => "Ratenlimit {value}",
        "Matched provider {id}" => "Passender Anbieter {id}",
        "Matched model {id}" => "Passendes Modell {id}",
        "Match strategy {strategy}" => "Passstrategie {strategy}",
        "Capability overrides {fields}" => "Fähigkeitsüberschreibungen {fields}",
        "yes" => "ja",
        "no" => "nein",
        "off" => "aus",
        "supported" => "unterstützt",
        _ => return None,
    })
}
