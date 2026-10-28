//! Interface settings in `<home>/config.json`, one top-level key each; a
//! write keeps the other keys.

use std::path::{Path, PathBuf};

use rutis::host::CtxHost;
use serde_json::Value;

/// The value saved under `key`, if any.
pub fn read(home: &Path, key: &str) -> Option<Value> {
    let text = std::fs::read_to_string(path(home)).ok()?;
    let mut value: Value = serde_json::from_str(&text).ok()?;
    value.get_mut(key).map(Value::take)
}

/// Saves `value` under `key`; the error text on failure.
pub fn write(home: &Path, key: &str, value: Value) -> Option<String> {
    use dim_agent_capabilities::file_storage::{FileStorage, FileStorageError, FileStoragePlugin};
    let home = home.to_owned();
    let key = key.to_owned();
    let worker = std::thread::Builder::new()
        .name("client-settings".into())
        .spawn(move || -> crate::Result<()> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async move {
                std::fs::create_dir_all(&home)?;
                let root = rutis::Ctx::root()?;
                let result = async {
                    let plugin = root.plugin(FileStoragePlugin::new(vec![home.canonicalize()?]));
                    (&plugin).await?;
                    let files = root
                        .get::<FileStorage>()
                        .ok_or("Client settings storage is unavailable")?;
                    let cancel = tokio_util::sync::CancellationToken::new();
                    let mut lease = files.lock(&files.path(0, "config.json")?, &cancel).await?;
                    let mut document: Value = match lease.read(&cancel).await {
                        Ok(bytes) => serde_json::from_slice(&bytes)?,
                        Err(FileStorageError::Io(error))
                            if error.kind() == std::io::ErrorKind::NotFound =>
                        {
                            serde_json::json!({})
                        }
                        Err(error) => return Err(error.into()),
                    };
                    let fields = document
                        .as_object_mut()
                        .ok_or("Client config.json must be an object")?;
                    fields.insert(key, value);
                    #[cfg(unix)]
                    let permissions = Some(std::os::unix::fs::PermissionsExt::from_mode(0o600));
                    #[cfg(not(unix))]
                    let permissions = None;
                    lease
                        .replace(serde_json::to_vec_pretty(&document)?, permissions, &cancel)
                        .await?;
                    Ok(())
                }
                .await;
                let cleanup = root.shutdown().await.map_err(Into::into);
                result.and(cleanup)
            })
        });
    match worker {
        Err(error) => Some(error.to_string()),
        Ok(worker) => match worker.join() {
            Ok(result) => result.err().map(|error| error.to_string()),
            Err(_) => Some("Client settings worker failed".into()),
        },
    }
}

/// Saves an explicit cleared value so import cannot replace the user's default choice.
pub fn clear(home: &Path, key: &str) -> Option<String> {
    write(home, key, Value::Null)
}

/// `config.json` key of the default terminal UI mode.
const TERMINAL_MODE_KEY: &str = "tuiMode";

/// True when the saved default terminal UI mode is fullscreen; scrollback
/// is the default when nothing was saved.
pub fn saved_fullscreen(home: &Path) -> bool {
    read(home, TERMINAL_MODE_KEY).is_some_and(|mode| mode == "fullscreen")
}

/// Saves the default terminal UI mode; the error text on failure.
pub fn save_fullscreen(home: &Path, fullscreen: bool) -> Option<String> {
    let mode = if fullscreen {
        "fullscreen"
    } else {
        "scrollback"
    };
    write(home, TERMINAL_MODE_KEY, mode.into())
}

const PERMISSION_KEY: &str = "permissionPreset";

/// The permission preset the user last chose in `/approvals`, the default
/// for the sessions the terminal UI creates or resumes.
pub fn saved_permission(home: &Path) -> Option<String> {
    read(home, PERMISSION_KEY).and_then(|value| value.as_str().map(str::to_owned))
}

/// Remembers the chosen preset; the error text on failure.
pub fn save_permission(home: &Path, preset: &str) -> Option<String> {
    write(home, PERMISSION_KEY, preset.into())
}

fn path(home: &Path) -> PathBuf {
    home.join("config.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dim_agent_capabilities::file_storage::{FileStorage, FileStoragePlugin};

    #[tokio::test]
    async fn client_writes_share_migration_locks_and_keep_explicit_clear_and_corrupt_files() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            path(home.path()),
            br#"{"locale":"en","tuiNotifyOnComplete":false}"#,
        )
        .unwrap();
        let root = rutis::Ctx::root().unwrap();
        let plugin = root.plugin(FileStoragePlugin::new(vec![
            home.path().canonicalize().unwrap(),
        ]));
        (&plugin).await.unwrap();
        let files = root.get::<FileStorage>().unwrap();
        let lease = files
            .lock(
                &files.path(0, "config.json").unwrap(),
                &tokio_util::sync::CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(write(home.path(), "locale", "ja".into()).is_some());
        assert_eq!(read(home.path(), "locale"), Some("en".into()));
        drop(lease);
        assert_eq!(clear(home.path(), "locale"), None);
        assert_eq!(read(home.path(), "locale"), Some(Value::Null));
        assert_eq!(read(home.path(), "tuiNotifyOnComplete"), Some(false.into()));
        std::fs::write(path(home.path()), b"broken document").unwrap();
        assert!(write(home.path(), "locale", "ja".into()).is_some());
        assert_eq!(
            std::fs::read(path(home.path())).unwrap(),
            b"broken document"
        );
        root.shutdown().await.unwrap();
    }
}
