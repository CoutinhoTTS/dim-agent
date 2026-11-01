pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
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
        "Hooks" => "Hooks",
        "invalid" => "tidak valid",
        "Saved." => "Disimpan.",
        "No change." => "Tidak ada perubahan.",
        "Your text is kept in {path}." => "Teks Anda tersimpan di {path}.",
        "The user's own document; always in effect." => "Dokumen milik pengguna; selalu berlaku.",
        "No .dimcode/hooks.json in this workspace." => {
            "Tidak ada .dimcode/hooks.json di ruang kerja ini."
        }
        "Owned by plugin {plugin}." => "Milik plugin {plugin}.",
        "The user document turns every hook off (disableAllHooks)." => {
            "Dokumen pengguna mematikan semua hook (disableAllHooks)."
        }
        "Enter toggle" => "Enter alih",
        "Ctrl+E edit" => "Ctrl+E edit",
        "Ctrl+N add" => "Ctrl+N tambah",
        "Ctrl+R reload" => "Ctrl+R muat ulang",
        "Ctrl+T run" => "Ctrl+T jalankan",
        "Ctrl+X remove" => "Ctrl+X hapus",
        "Tab inspect" => "Tab periksa",
        "Remove {name}? Enter confirm · Esc cancel" => {
            "Hapus {name}? Enter konfirmasi · Esc batal"
        }
        "↑↓ scroll · Esc back" => "↑↓ gulir · Esc kembali",
        "Added." => "Ditambahkan.",
        "Removed {name}." => "{name} dihapus.",
        "Ran {name}." => "{name} dijalankan.",
        "The user document changed since it was read; nothing was written. Your next edit opens the current document." => {
            "Dokumen pengguna berubah sejak dibaca; tidak ada yang ditulis. Edit berikutnya membuka dokumen saat ini."
        }
        _ => return None,
    })
}
