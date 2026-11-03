use super::{SessionStore, sqlite::error};
use dim_agent_runtime::{AgentError, Result};
use rusqlite::{TransactionBehavior, params};
use std::path::{Component, Path, PathBuf};

/// A recorded worktree directory, identified by its canonical Git common
/// directory and root. This is usage metadata, not permission or ownership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeDirectory {
    pub repository: PathBuf,
    pub root: PathBuf,
}

/// A Session's verified subtree in one recorded worktree. The association
/// survives close, is removed by deletion, and is not copied by a fork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeUsage {
    pub session_id: String,
    pub subtree: PathBuf,
}

impl SessionStore {
    /// Atomically records a Git-verified directory and its Session association.
    /// Paths must be canonical absolute UTF-8 paths and `subtree` must be inside
    /// the root. Repeated calls replace this Session's subtree. A missing or
    /// deleted Session fails as Closed; failure changes neither record.
    pub async fn associate_worktree(
        &self,
        session_id: &str,
        directory: WorktreeDirectory,
        subtree: PathBuf,
    ) -> Result<()> {
        let repository = stored_path(&directory.repository)?.to_owned();
        let root = stored_path(&directory.root)?.to_owned();
        let subtree_text = stored_path(&subtree)?.to_owned();
        if !subtree.starts_with(&directory.root) {
            return Err(error(
                "the inherited subtree must be inside the worktree root",
            ));
        }
        let _ownership = self.ownership.lock().await;
        let session = session_id.to_owned();
        self.timeline
            .db
            .run(move |connection| {
                let tx = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(error)?;
                let exists: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE session_id=?1)",
                        [&session],
                        |row| row.get(0),
                    )
                    .map_err(error)?;
                if !exists {
                    return Err(AgentError::Closed(session));
                }
                tx.execute(
                    "INSERT INTO worktree_directories (repository, root) VALUES (?1, ?2) \
                     ON CONFLICT (repository, root) DO NOTHING",
                    params![repository, root],
                )
                .map_err(error)?;
                tx.execute(
                    "INSERT INTO session_worktrees (session_id, repository, root, subtree) \
                     VALUES (?1, ?2, ?3, ?4) ON CONFLICT (session_id, repository, root) \
                     DO UPDATE SET subtree=excluded.subtree",
                    params![session, repository, root, subtree_text],
                )
                .map_err(error)?;
                tx.commit().map_err(error)
            })
            .await
    }

    /// Lists recorded directories, including those with no Session association.
    /// The result is not a Git inventory or authority to delete any directory.
    pub async fn worktree_directories(&self) -> Result<Vec<WorktreeDirectory>> {
        self.timeline
            .db
            .run_read(|connection| {
                connection
                    .prepare("SELECT repository, root FROM worktree_directories ORDER BY repository, root")
                    .map_err(error)?
                    .query_map([], |row| {
                        Ok(WorktreeDirectory {
                            repository: PathBuf::from(row.get::<_, String>(0)?),
                            root: PathBuf::from(row.get::<_, String>(1)?),
                        })
                    })
                    .map_err(error)?
                    .collect::<rusqlite::Result<_>>()
                    .map_err(error)
            })
            .await
    }

    /// Lists the Session associations of a recorded directory. An unknown
    /// directory or a candidate without remaining associations has no usages.
    pub async fn worktree_usages(
        &self,
        directory: WorktreeDirectory,
    ) -> Result<Vec<WorktreeUsage>> {
        let repository = stored_path(&directory.repository)?.to_owned();
        let root = stored_path(&directory.root)?.to_owned();
        self.timeline
            .db
            .run_read(move |connection| {
                connection
                    .prepare(
                        "SELECT session_id, subtree FROM session_worktrees \
                         WHERE repository=?1 AND root=?2 ORDER BY session_id",
                    )
                    .map_err(error)?
                    .query_map(params![repository, root], |row| {
                        Ok(WorktreeUsage {
                            session_id: row.get(0)?,
                            subtree: PathBuf::from(row.get::<_, String>(1)?),
                        })
                    })
                    .map_err(error)?
                    .collect::<rusqlite::Result<_>>()
                    .map_err(error)
            })
            .await
    }
}

fn stored_path(path: &Path) -> Result<&str> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(error(
            "a worktree path must be absolute without relative components",
        ));
    }
    path.to_str()
        .ok_or_else(|| error("a worktree path must be UTF-8"))
}
