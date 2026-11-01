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
        "Command rules" => "Aturan perintah",
        "user" => "pengguna",
        "Matches only this command." => "Hanya cocok dengan perintah ini.",
        "Matches commands whose first words are these." => {
            "Cocok dengan perintah yang kata awalnya ini."
        }
        "Allows a simple command that would otherwise ask." => {
            "Mengizinkan perintah sederhana yang biasanya ditanyakan."
        }
        "Denies the command under every preset." => "Menolak perintah itu pada setiap preset.",
        "Applies in every Session, in every workspace." => {
            "Berlaku di setiap Sesi, di setiap workspace."
        }
        "Rule added: {rule}." => "Aturan ditambahkan: {rule}.",
        "Rule already present: {rule}." => "Aturan sudah ada: {rule}.",
        "Rule removed: {rule}." => "Aturan dihapus: {rule}.",
        "That command rule is no longer listed; nothing was removed." => {
            "Aturan perintah itu tidak lagi terdaftar; tidak ada yang dihapus."
        }
        "Command rules changed since they were read; nothing was removed." => {
            "Aturan perintah berubah sejak dibaca; tidak ada yang dihapus."
        }
        "A command rule is DECISION MATCH COMMAND, such as deny prefix git push." => {
            "Aturan perintah ditulis DECISION MATCH COMMAND, misalnya deny prefix git push."
        }
        "Use the line prompt or dimcode permissions." => {
            "Gunakan prompt baris atau dimcode permissions."
        }
        _ => return None,
    })
}
