pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Enter toggle" => "Enter alternar",
        "Ctrl+R reload" => "Ctrl+R recarregar",
        "Enter toggle · Ctrl+R reload · Esc close" => {
            "Enter alternar · Ctrl+R recarregar · Esc fechar"
        }
        "Loading..." => "Carregando…",
        "No entries." => "Nenhuma entrada.",
        "No matching entry." => "Nenhuma entrada correspondente.",
        "Issues" => "Problemas",
        "enabled" => "ativado",
        "shadowed by {name}" => "ocultado por {name}",
        "Reloaded." => "Recarregado.",
        "Enabled {name}." => "{name} ativado.",
        "Disabled {name}." => "{name} desativado.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} recarregado: {entries} entradas, {disabled} desativadas."
        }
        _ => return None,
    })
}
