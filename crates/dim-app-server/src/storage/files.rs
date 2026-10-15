//! SessionStore's Session files ([design](../../../../docs/design/appserver/session-files.md)):
//! immutable files at `<db>.session-files/<fileId>/<name>` on the Session's
//! resource node, owned by `session_file_owners` rows in the native store.
//!
//! A put is two write transactions around the file write. Txn 1 records the
//! file as pending under this SessionStore generation `G`; txn 2 makes it
//! owned only while the Session exists and the row is still pending under
//! `G`. SQLite orders both against S5c's fork and delete, so no in-memory
//! lock is needed.
use super::{SqliteStorage, sqlite::error};
use dim_agent_capabilities::{
    file_storage::{FileStorage, FileStorageError},
    host::{PutSessionFile, SessionFileError, SessionFileErrorCode, StoredSessionFile},
};
use rusqlite::{Connection, TransactionBehavior, params};
use rutis::Ctx;
use std::{
    ffi::OsString,
    fs::Permissions,
    future::Future,
    path::Path,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// One file's limit: a put is one message, and storage holds it in memory.
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// The `sessionFiles` service of one SessionStore generation.
#[derive(Clone)]
pub(super) struct SessionFiles(Arc<Files>);
struct Files {
    db: Arc<SqliteStorage>,
    /// SessionStore's own context: each call reads the node's FileStorage,
    /// so a FileStorage reload reloads nothing above it.
    ctx: Ctx,
    /// The FileStorage root of the database directory.
    root: usize,
    /// `<db>.session-files`, the directory below that root.
    directory: OsString,
    /// This generation's writer id.
    generation: String,
    admission: Mutex<()>,
    stopping: CancellationToken,
    tasks: TaskTracker,
}

fn file_error(code: SessionFileErrorCode, message: impl std::fmt::Display) -> SessionFileError {
    SessionFileError {
        code,
        message: message.to_string(),
    }
}
fn cancelled() -> SessionFileError {
    file_error(
        SessionFileErrorCode::Cancelled,
        "Session file put was cancelled",
    )
}
fn not_found() -> SessionFileError {
    file_error(SessionFileErrorCode::NotFound, "Session not found")
}
fn node_unavailable() -> SessionFileError {
    file_error(
        SessionFileErrorCode::NodeUnavailable,
        "the Session's resource node storage is unavailable",
    )
}
/// A FileStorage failure while the file is written.
fn storage_failure(failure: FileStorageError) -> SessionFileError {
    match failure {
        FileStorageError::Closed => node_unavailable(),
        FileStorageError::Cancelled => cancelled(),
        other => file_error(SessionFileErrorCode::Storage, other),
    }
}

/// `[A-Za-z0-9._-]{1,64}` without a leading dot, never FileStorage's account
/// file or marker in any letter case: `auth.json` would switch replacement to
/// the account-marker protocol.
fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !name.eq_ignore_ascii_case("auth.json")
        && !name.eq_ignore_ascii_case("auth.json.lock")
}

/// Written once, read-only: a guard against `write` and `edit`, not isolation.
#[cfg(unix)]
fn read_only(_: &Path) -> std::io::Result<Permissions> {
    use std::os::unix::fs::PermissionsExt;
    Ok(Permissions::from_mode(0o400))
}
/// Windows permissions carry file attributes and come only from metadata; the
/// database file is a regular file in the same root.
#[cfg(not(unix))]
fn read_only(database: &Path) -> std::io::Result<Permissions> {
    let mut permissions = std::fs::metadata(database)?.permissions();
    permissions.set_readonly(true);
    Ok(permissions)
}

fn session_exists(connection: &Connection, session_id: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE session_id = ?1)",
        [session_id],
        |row| row.get(0),
    )
}

/// How txn 2 ended.
enum Commit {
    Owned,
    SessionGone,
    /// A sweep of a later generation claimed the row.
    Superseded,
}

impl SessionFiles {
    pub(super) fn new(db: Arc<SqliteStorage>, ctx: Ctx, root: usize) -> Self {
        let mut directory = db
            .path
            .file_name()
            .expect("the native store path names a file")
            .to_os_string();
        directory.push(".session-files");
        Self(Arc::new(Files {
            db,
            ctx,
            root,
            directory,
            generation: uuid::Uuid::new_v4().to_string(),
            admission: Mutex::new(()),
            stopping: CancellationToken::new(),
            tasks: TaskTracker::new(),
        }))
    }

    /// Cancels admitted puts and waits until each has committed or abandoned
    /// its file.
    pub(super) async fn close(&self) {
        {
            let _admission = self.0.admission.lock().unwrap();
            self.0.stopping.cancel();
            self.0.tasks.close();
        }
        self.0.tasks.wait().await;
    }
}

impl PutSessionFile for SessionFiles {
    fn put(
        &self,
        session_id: String,
        name: String,
        content: Vec<u8>,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<StoredSessionFile, SessionFileError>> + Send + '_>>
    {
        Box::pin(async move {
            if !valid_name(&name) {
                return Err(file_error(
                    SessionFileErrorCode::Invalid,
                    "a Session file name is 1 to 64 of A-Z, a-z, 0-9, '.', '_' and '-', without a leading dot, and not auth.json",
                ));
            }
            if content.len() > MAX_BYTES {
                return Err(file_error(
                    SessionFileErrorCode::TooLarge,
                    "a Session file holds at most 16 MiB",
                ));
            }
            // The put runs as SessionStore's own task, so a caller that stops
            // waiting still leaves its file committed or abandoned, never
            // pending under a live generation. Dropping the call cancels it.
            let call = self.0.stopping.child_token();
            let mut task = {
                let _admission = self.0.admission.lock().unwrap();
                if self.0.stopping.is_cancelled() {
                    return Err(cancelled());
                }
                let files = self.0.clone();
                let call = call.clone();
                self.0
                    .tasks
                    .spawn(async move { files.put(session_id, name, content, call).await })
            };
            let _dropped = call.clone().drop_guard();
            let result = tokio::select! {
                result = &mut task => result,
                () = cancel.cancelled() => {
                    call.cancel();
                    task.await
                }
            };
            result.unwrap_or_else(|join| Err(file_error(SessionFileErrorCode::Storage, join)))
        })
    }
}

impl Files {
    async fn put(
        self: Arc<Self>,
        session_id: String,
        name: String,
        content: Vec<u8>,
        cancel: CancellationToken,
    ) -> Result<StoredSessionFile, SessionFileError> {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let storage = self.ctx.get::<FileStorage>().ok_or_else(node_unavailable)?;
        let file_id = uuid::Uuid::new_v4().to_string();
        let file_directory = Path::new(&self.directory).join(&file_id);
        let paths = (|| {
            Ok::<_, FileStorageError>((
                storage.path(self.root, &self.directory)?,
                storage.path(self.root, &file_directory)?,
                storage.path(self.root, file_directory.join(&name))?,
                storage.root_path(self.root)?,
            ))
        })();
        let (directory, file_path, file, root) = match paths {
            Ok(paths) => paths,
            Err(FileStorageError::Closed) => return Err(node_unavailable()),
            // The name is not one component on this node, as a Windows
            // device name is not.
            Err(other) => return Err(file_error(SessionFileErrorCode::Invalid, other)),
        };
        let permissions = read_only(&self.db.path)
            .map_err(|failure| file_error(SessionFileErrorCode::Storage, failure))?;

        // Txn 1: the row commits before its directory exists, so a sweep that
        // lists the directory finds the row of every entry a put created.
        let (session, id, generation) =
            (session_id.clone(), file_id.clone(), self.generation.clone());
        let pending = self
            .db
            .run(move |connection| {
                let tx = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(error)?;
                if !session_exists(&tx, &session).map_err(error)? {
                    return Ok(false);
                }
                tx.execute(
                    "INSERT INTO session_files (file_id, node, writer) VALUES (?1, NULL, ?2)",
                    params![id, generation],
                )
                .map_err(error)?;
                tx.commit().map_err(error)?;
                Ok(true)
            })
            .await
            .map_err(|failure| file_error(SessionFileErrorCode::Storage, failure))?;
        if !pending {
            return Err(not_found());
        }

        let bytes = content.len() as u64;
        let written = async {
            storage.create_directory(&directory, &cancel).await?;
            storage.create_directory(&file_path, &cancel).await?;
            storage
                .replace(&file, content, Some(permissions), &cancel)
                .await
        }
        .await;
        if let Err(failure) = written {
            self.abandon(&file_id).await;
            return Err(storage_failure(failure));
        }
        if cancel.is_cancelled() {
            self.abandon(&file_id).await;
            return Err(cancelled());
        }

        // Txn 2: the Session still exists and the row is still ours.
        let (session, id, generation) = (session_id, file_id.clone(), self.generation.clone());
        let committed = self
            .db
            .run(move |connection| {
                let tx = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(error)?;
                if !session_exists(&tx, &session).map_err(error)? {
                    return Ok(Commit::SessionGone);
                }
                if tx
                    .execute(
                        "UPDATE session_files SET writer = NULL WHERE file_id = ?1 AND writer = ?2",
                        params![id, generation],
                    )
                    .map_err(error)?
                    == 0
                {
                    return Ok(Commit::Superseded);
                }
                tx.execute(
                    "INSERT INTO session_file_owners (session_id, file_id) VALUES (?1, ?2)",
                    params![session, id],
                )
                .map_err(error)?;
                tx.commit().map_err(error)?;
                Ok(Commit::Owned)
            })
            .await;
        match committed {
            Ok(Commit::Owned) => Ok(StoredSessionFile {
                path: root.join(&file_directory).join(&name),
                file_id,
                bytes,
            }),
            Ok(Commit::SessionGone) => {
                self.abandon(&file_id).await;
                Err(not_found())
            }
            Ok(Commit::Superseded) => Err(cancelled()),
            Err(failure) => {
                self.abandon(&file_id).await;
                Err(file_error(SessionFileErrorCode::Storage, failure))
            }
        }
    }

    /// Makes a pending file of this generation unowned. If that fails too,
    /// the row stays pending under this generation and the next one sweeps it.
    async fn abandon(&self, file_id: &str) {
        let (id, generation) = (file_id.to_owned(), self.generation.clone());
        let abandoned = self
            .db
            .run(move |connection| {
                connection
                    .execute(
                        "UPDATE session_files SET writer = NULL WHERE file_id = ?1 AND writer = ?2",
                        params![id, generation],
                    )
                    .map_err(error)
            })
            .await;
        if let Err(failure) = abandoned {
            tracing::warn!(file_id, error = %failure, "Session file left pending for the next generation");
        }
    }
}
