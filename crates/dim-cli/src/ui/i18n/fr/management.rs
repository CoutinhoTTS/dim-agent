pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Enter toggle" => "Enter basculer",
        "Ctrl+R reload" => "Ctrl+R recharger",
        "Enter toggle · Ctrl+R reload · Esc close" => {
            "Enter basculer · Ctrl+R recharger · Esc fermer"
        }
        "Loading..." => "Chargement…",
        "No entries." => "Aucune entrée.",
        "No matching entry." => "Aucune entrée correspondante.",
        "Issues" => "Problèmes",
        "enabled" => "activé",
        "shadowed by {name}" => "masqué par {name}",
        "Reloaded." => "Rechargé.",
        "Enabled {name}." => "{name} activé.",
        "Disabled {name}." => "{name} désactivé.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} rechargé : {entries} entrées, {disabled} désactivées."
        }
        _ => return None,
    })
}
