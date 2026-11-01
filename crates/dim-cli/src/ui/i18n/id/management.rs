pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "Memuat…",
        "No entries." => "Tidak ada entri.",
        "No matching entry." => "Tidak ada entri yang cocok.",
        "Issues" => "Masalah",
        "sources" => "Sumber",
        "skills" => "Skill",
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
        "Scheduler" => "Penjadwal",
        "Added {name}." => "{name} ditambahkan.",
        "Run {name}: {outcome}." => "{name} dijalankan: {outcome}.",
        "Recent runs of {name}" => "Eksekusi terbaru {name}",
        "No run yet." => "Belum ada eksekusi.",
        "No next run." => "Tidak ada eksekusi berikutnya.",
        "Next run {instant}." => "Eksekusi berikutnya {instant}.",
        "Last run {outcome} at {instant}." => "Eksekusi terakhir {outcome} pada {instant}.",
        "every {minutes} minutes" => "setiap {minutes} menit",
        "at {instant}" => "pada {instant}",
        "cron {expression}" => "cron {expression}",
        "cron {expression} in {zone}" => "cron {expression} di {zone}",
        "The job changed since it was read; nothing was written. Your next edit opens the current job." => {
            "Tugas berubah sejak dibaca; tidak ada yang ditulis. Edit berikutnya membuka tugas saat ini."
        }
        "The jobs changed since they were read; nothing was written. Your next add opens your text again." => {
            "Tugas berubah sejak dibaca; tidak ada yang ditulis. Tambah berikutnya membuka kembali teks Anda."
        }
        "The job changed since it was read; nothing was removed." => {
            "Tugas berubah sejak dibaca; tidak ada yang dihapus."
        }
        "The Scheduler has no area document." => "Penjadwal tidak memiliki dokumen area.",
        "No scheduled job has that id." => "Tidak ada tugas terjadwal dengan id itu.",
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "Pengaturan hook ruang kerja ini berubah sejak dibaca; tidak ada yang ditulis."
        }
        "Workflow" => "Alur kerja",
        "Saved {name}." => "{name} disimpan.",
        "Saved {name}. It stays disabled until you enable it." => {
            "{name} disimpan. Tetap nonaktif sampai Anda mengaktifkannya."
        }
        "The definitions changed since they were read; nothing was written." => {
            "Definisi berubah sejak dibaca; tidak ada yang ditulis."
        }
        "The definition changed since it was read; nothing was written. Your next edit opens the current definition." => {
            "Definisi berubah sejak dibaca; tidak ada yang ditulis. Edit berikutnya membuka definisi saat ini."
        }
        "The definitions changed since they were read; nothing was written. Your next add opens your text again." => {
            "Definisi berubah sejak dibaca; tidak ada yang ditulis. Penambahan berikutnya membuka teks Anda lagi."
        }
        "A definition's name is its key; add it to save it under another name." => {
            "Nama definisi adalah kuncinya; tambahkan untuk menyimpannya dengan nama lain."
        }
        "No project definition is named {name}." => "Tidak ada definisi proyek bernama {name}.",
        "No workflow definition is named {name}." => {
            "Tidak ada definisi alur kerja bernama {name}."
        }
        "Saved in the user workflow source; always enabled." => {
            "Disimpan di sumber alur kerja pengguna; selalu aktif."
        }
        "Starts only after you enable it." => "Hanya berjalan setelah Anda mengaktifkannya.",
        "Defined by package {package}." => "Didefinisikan oleh paket {package}.",
        "{name} — {description} (required)" => "{name} — {description} (wajib)",
        "{name} — {description}" => "{name} — {description}",
        "{name} — required" => "{name} — wajib",
        "{name} — default: {default}" => "{name} — default: {default}",
        "and {count} more" => "dan {count} lainnya",
        "Recent runs" => "Jalan terbaru",
        "A workflow definition is saved as an object with a source and a document." => {
            "Definisi alur kerja disimpan sebagai objek dengan sumber dan dokumen."
        }
        "A definition is saved to the user source or the project source." => {
            "Definisi disimpan ke sumber pengguna atau sumber proyek."
        }
        "Started {name}: {run_id}." => "{name} dimulai: {run_id}.",
        "Parameters are a JSON object of strings." => "Parameter adalah objek JSON berisi string.",
        _ => return None,
    })
}
