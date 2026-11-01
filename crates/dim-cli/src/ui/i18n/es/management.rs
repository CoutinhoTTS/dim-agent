pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Enter toggle" => "Enter alternar",
        "Ctrl+R reload" => "Ctrl+R recargar",
        "Enter toggle · Ctrl+R reload · Esc close" => {
            "Enter alternar · Ctrl+R recargar · Esc cerrar"
        }
        "Loading..." => "Cargando…",
        "No entries." => "No hay entradas.",
        "No matching entry." => "Ninguna entrada coincide.",
        "Issues" => "Problemas",
        "enabled" => "activado",
        "shadowed by {name}" => "ocultado por {name}",
        "Reloaded." => "Recargado.",
        "Enabled {name}." => "{name} activado.",
        "Disabled {name}." => "{name} desactivado.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} recargado: {entries} entradas, {disabled} desactivadas."
        }
        _ => return None,
    })
}
