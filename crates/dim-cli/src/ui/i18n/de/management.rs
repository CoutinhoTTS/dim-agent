pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Enter toggle" => "Enter umschalten",
        "Ctrl+R reload" => "Ctrl+R neu laden",
        "Enter toggle · Ctrl+R reload · Esc close" => {
            "Enter umschalten · Ctrl+R neu laden · Esc schließen"
        }
        "Loading..." => "Wird geladen…",
        "No entries." => "Keine Einträge.",
        "No matching entry." => "Kein passender Eintrag.",
        "Issues" => "Probleme",
        "enabled" => "aktiviert",
        "shadowed by {name}" => "von {name} verdeckt",
        "Reloaded." => "Neu geladen.",
        "Enabled {name}." => "{name} aktiviert.",
        "Disabled {name}." => "{name} deaktiviert.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} neu geladen: {entries} Einträge, {disabled} deaktiviert."
        }
        _ => return None,
    })
}
