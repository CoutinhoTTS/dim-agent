//! Replacing a file whole, under a cross-process lock.

#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// Replaces the file at `path` with the bytes `update` returns for its
/// current contents, or leaves it alone when `update` returns `None`.
/// Writers queue on an exclusive lock over `<path>.lock`, so two processes
/// keep each other's changes; the new bytes go to a sibling temp file that
/// is synced and renamed over `path`, so a reader, which takes no lock,
/// sees either the old file or the new one. On Unix `mode`, when given, is
/// set on every write; without one an existing file keeps its mode. A
/// failure before the rename leaves `path` as it was and removes the temp
/// file, and a file that cannot be read is left alone rather than treated
/// as an absent one.
pub fn update(
    path: &Path,
    mode: Option<u32>,
    update: impl FnOnce(Option<&[u8]>) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(sibling(path, ".lock"))?;
    lock.lock()?;
    let current = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let Some(bytes) = update(current.as_deref())? else {
        return Ok(());
    };
    let temp = sibling(path, &format!(".{}.tmp", std::process::id()));
    let replaced = write_temp(&temp, path, mode, &bytes)
        .and_then(|()| std::fs::rename(&temp, path))
        .and_then(|()| sync_dir(path));
    if replaced.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    replaced
}

fn write_temp(temp: &Path, path: &Path, mode: Option<u32>, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(temp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = mode.or_else(|| {
            std::fs::metadata(path)
                .ok()
                .map(|meta| meta.permissions().mode() & 0o777)
        });
        if let Some(mode) = mode {
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    file.write_all(bytes)?;
    file.sync_all()
}

/// A rename is durable once its directory is synced.
fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}
