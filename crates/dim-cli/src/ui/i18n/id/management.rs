pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Loading..." => "Memuat…",
        "No entries." => "Tidak ada entri.",
        "No matching entry." => "Tidak ada entri yang cocok.",
        "Issues" => "Masalah",
        "sources" => "Sumber",
        "skills" => "Skill",
        "hooks" => "hook",
        "scheduled jobs" => "tugas terjadwal",
        "workflow definitions" => "definisi alur kerja",
        "command rules" => "Aturan perintah",
        "enabled" => "aktif",
        "shadowed by {name}" => "tertutup oleh {name}",
        "Reloaded." => "Dimuat ulang.",
        "Enabled {name}." => "{name} diaktifkan.",
        "Disabled {name}." => "{name} dinonaktifkan.",
        "Reloaded {area}: {entries} entries, {disabled} disabled." => {
            "{area} dimuat ulang: {entries} entri, {disabled} nonaktif."
        }
        "Reloaded {area}: {entries} entries." => "{area} dimuat ulang: {entries} entri.",
        "Hooks" => "Hooks",
        "invalid" => "tidak valid",
        "Saved." => "Disimpan.",
        "Saved {name}." => "{name} disimpan.",
        "No change." => "Tidak ada perubahan.",
        "Your text is kept in {path}." => "Teks Anda tersimpan di {path}.",
        "The user's own document; always in effect." => "Dokumen milik pengguna; selalu berlaku.",
        "There is no hooks file there." => "Tidak ada file hook di sana.",
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
        "The user document changed since it was read; nothing was written." => {
            "Dokumen pengguna berubah sejak dibaca; tidak ada yang ditulis."
        }
        "The job changed since it was read; nothing was written." => {
            "Tugas berubah sejak dibaca; tidak ada yang ditulis."
        }
        "The jobs changed since they were read; nothing was written." => {
            "Tugas berubah sejak dibaca; tidak ada yang ditulis."
        }
        "Your next edit opens the current document." => "Edit berikutnya membuka dokumen saat ini.",
        "Your next add reopens your text." => "Tambah berikutnya membuka kembali teks Anda.",
        "Scheduler" => "Penjadwal",
        "jobs" => "tugas",
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
        "The job changed since it was read; nothing was removed." => {
            "Tugas berubah sejak dibaca; tidak ada yang dihapus."
        }
        "The Scheduler has no area document." => "Penjadwal tidak memiliki dokumen area.",
        "No scheduled job has that id." => "Tidak ada tugas terjadwal dengan id itu.",
        "A scheduled job's run takes no input document." => {
            "Eksekusi job terjadwal tidak menerima dokumen masukan."
        }
        "This daemon cannot manage {area}." => "Tidak dapat mengelola {area}.",
        "This daemon lists {area} but cannot toggle them." => {
            "Daemon ini menampilkan {area}, tetapi tidak dapat mengalihkannya."
        }
        "This daemon lists {area} but cannot edit them." => {
            "Daemon ini menampilkan {area}, tetapi tidak dapat mengeditnya."
        }
        "This daemon lists {area} but cannot reload them." => {
            "Daemon ini menampilkan {area}, tetapi tidak dapat memuat ulang daftarnya."
        }
        "This daemon lists {area} but cannot add them." => {
            "Daemon ini menampilkan {area}, tetapi tidak dapat menambahnya."
        }
        "This daemon lists {area} but cannot remove them." => {
            "Daemon ini menampilkan {area}, tetapi tidak dapat menghapusnya."
        }
        "This daemon lists {area} but cannot run them." => {
            "Daemon ini menampilkan {area}, tetapi tidak dapat menjalankannya."
        }
        "This daemon lists {area} but cannot inspect them." => {
            "Daemon ini menampilkan {area}, tetapi tidak dapat melihat detail atau riwayatnya."
        }
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "Dokumen menghapus {field}, yang tidak dapat dibersihkan pemilik; tidak ada yang ditulis."
        }
        "The document changes no field; nothing was written." => {
            "Dokumen tidak mengubah bidang apa pun; tidak ada yang ditulis."
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "Pengaturan hook ruang kerja ini berubah sejak dibaca; tidak ada yang ditulis."
        }
        "Workflow" => "Alur kerja",
        "Saved {name}. It stays disabled until you enable it." => {
            "{name} disimpan. Tetap nonaktif sampai Anda mengaktifkannya."
        }
        "The definitions changed since they were read; nothing was written." => {
            "Definisi berubah sejak dibaca; tidak ada yang ditulis."
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
        "Command: {command}" => "Perintah: {command}",
        "Use the line prompt or dimcode permissions." => {
            "Gunakan prompt baris atau dimcode permissions."
        }
        _ => return None,
    })
}
