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
        "Saved {name}." => "{name} disimpan.",
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
        "unavailable" => "tidak tersedia",
        "Reset {name}? Enter confirm · Esc cancel" => "Reset {name}? Enter konfirmasi · Esc batal",
        "Reset {name} to its catalog entry." => "{name} direset ke entri katalog.",
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
        "This daemon cannot manage {area}." => "Tidak dapat mengelola {area}.",
        "The document deleted {field}, which the owner cannot clear; nothing was written." => {
            "Dokumen menghapus {field}, yang tidak dapat dibersihkan pemilik; tidak ada yang ditulis."
        }
        "The document changes no field; nothing was written." => {
            "Dokumen tidak mengubah bidang apa pun; tidak ada yang ditulis."
        }
        "This workspace's hook settings changed since they were read; nothing was written." => {
            "Pengaturan hook ruang kerja ini berubah sejak dibaca; tidak ada yang ditulis."
        }
        "Models" => "Model",
        "catalog" => "katalog",
        "overrides" => "penimpaan",
        "added" => "ditambahkan",
        "Models · {name}" => "Model · {name}",
        "undeclared" => "belum dideklarasikan",
        "Context window {window} · max output {output}" => {
            "Jendela konteks {window} · output maks {output}"
        }
        "Endpoint {endpoint}" => "Endpoint {endpoint}",
        "Endpoint {endpoint} · override {value}" => "Endpoint {endpoint} · penimpaan {value}",
        "The provider reports it unavailable." => "Penyedia melaporkannya tidak tersedia.",
        "Limits undeclared; edit the model to declare them." => {
            "Batas belum dideklarasikan; edit modelnya untuk mendeklarasikan."
        }
        "Sign in to the account to use its models." => "Masuk ke akun untuk memakai modelnya.",
        "The signed-in account changed; reload to list its models." => {
            "Akun yang masuk berubah; muat ulang untuk menampilkan modelnya."
        }
        "The provider has no catalog yet." => "Penyedia ini belum punya katalog.",
        "The provider listed no models." => "Penyedia tidak menampilkan model apa pun.",
        "Listed {count} models." => "Menampilkan {count} model.",
        "Refreshed the models." => "Model diperbarui.",
        "Could not list models: {error}. Showing the stored list." => {
            "Tidak dapat menampilkan model: {error}. Menampilkan daftar tersimpan."
        }
        "The signed-in account changed; the refresh cleared preferences, overrides and disables." => {
            "Akun yang masuk berubah; muat ulang menghapus preferensi, penimpaan, dan penonaktifan."
        }
        "The models changed since they were read; nothing was removed." => {
            "Model berubah sejak dibaca; tidak ada yang dihapus."
        }
        "The models changed since they were read; nothing was written." => {
            "Model berubah sejak dibaca; tidak ada yang ditulis."
        }
        "No provider has that id." => "Tidak ada penyedia dengan id itu.",
        "No model has that id." => "Tidak ada model dengan id itu.",
        "This model cannot be removed." => "Model ini tidak dapat dihapus.",
        "A Provider's models have no area document." => "Model penyedia tidak punya dokumen area.",
        "Models have no run operation." => "Model tidak punya operasi jalankan.",
        "The document is not an object; nothing was written." => {
            "Dokumen bukan objek; tidak ada yang ditulis."
        }
        "The document changes no editable field; nothing was written." => {
            "Dokumen tidak mengubah bidang yang dapat diedit; tidak ada yang ditulis."
        }
        "The model document names no model." => "Dokumen model tidak menamai model.",
        "The provider has no model catalog to write." => {
            "Penyedia ini tidak punya katalog model untuk ditulis."
        }
        "ID {id}" => "ID {id}",
        "Source {source}" => "Sumber {source}",
        "Status {status}" => "Status {status}",
        "Tools {value}" => "Alat {value}",
        "Vision {value}" => "Visi {value}",
        "Reasoning {value}" => "Penalaran {value}",
        "Modalities {list}" => "Modalitas {list}",
        "Description {text}" => "Deskripsi {text}",
        "Release date {date}" => "Tanggal rilis {date}",
        "Pricing {value}" => "Harga {value}",
        "Rate limit {value}" => "Batas laju {value}",
        "Matched provider {id}" => "Penyedia cocok {id}",
        "Matched model {id}" => "Model cocok {id}",
        "Match strategy {strategy}" => "Strategi cocok {strategy}",
        "Capability overrides {fields}" => "Penimpaan kemampuan {fields}",
        "yes" => "ya",
        "no" => "tidak",
        "off" => "mati",
        "supported" => "didukung",
        _ => return None,
    })
}
