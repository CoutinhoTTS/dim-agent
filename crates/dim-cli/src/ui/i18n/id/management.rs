pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Enter toggle" => "Enter alih",
        "Ctrl+R reload" => "Ctrl+R muat ulang",
        "Enter toggle · Ctrl+R reload · Esc close" => {
            "Enter alih · Ctrl+R muat ulang · Esc tutup"
        }
        "Loading..." => "Memuat…",
        "No entries." => "Tidak ada entri.",
        "No matching entry." => "Tidak ada entri yang cocok.",
        "Issues" => "Masalah",
        "enabled" => "aktif",
        "shadowed by {name}" => "tertutup oleh {name}",
        "Reloaded." => "Dimuat ulang.",
        "Enabled {name}." => "{name} diaktifkan.",
        "Disabled {name}." => "{name} dinonaktifkan.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} dimuat ulang: {entries} entri, {disabled} nonaktif."
        }
        _ => return None,
    })
}
