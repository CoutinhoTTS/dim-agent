//! Project MCP execution grants in the host's private `mcp-approvals.json`
//! (version 1): hashed workspace/server identities mapped to configuration
//! fingerprints, never commands, environment values or tokens. MCP owns this
//! record; Settings provides no approval service.
use crate::settings::{Result, SettingsError};
use dim_agent_capabilities::file_storage::{FileLease, FileStorage, FileStorageError};
use dim_agent_runtime::StateCell;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

const FILE: &str = "mcp-approvals.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grants {
    version: u32,
    approved: BTreeMap<String, String>,
}
fn decode(bytes: std::result::Result<Vec<u8>, FileStorageError>) -> Result<Grants> {
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(FileStorageError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Grants {
                version: 1,
                approved: BTreeMap::new(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let grants: Grants = serde_json::from_slice(&bytes)
        .map_err(|_| SettingsError::Invalid("invalid MCP approval store"))?;
    let digest = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    };
    if grants.version != 1
        || grants
            .approved
            .iter()
            .any(|(key, value)| !digest(key) || !digest(value))
    {
        return Err(SettingsError::Invalid("invalid MCP approval store"));
    }
    Ok(grants)
}
fn private_permissions() -> Option<std::fs::Permissions> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

pub(super) struct Approvals {
    files: Arc<FileStorage>,
    home_root: usize,
    stopping: CancellationToken,
    changed: StateCell<u64>,
}
impl Approvals {
    pub(super) fn new(
        files: Arc<FileStorage>,
        home_root: usize,
        stopping: CancellationToken,
    ) -> Self {
        Self {
            files,
            home_root,
            stopping,
            changed: StateCell::new(0),
        }
    }
    /// Changes after each persisted grant update.
    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
    pub(super) async fn approved(
        &self,
        key: &str,
        fingerprint: &str,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        let path = self.files.path(self.home_root, FILE)?;
        Ok(decode(self.files.read(&path, cancel).await)?
            .approved
            .get(key)
            .is_some_and(|value| value == fingerprint))
    }
    /// Only the publication worker writes, so updates are serialized.
    pub(super) async fn set(
        &self,
        key: String,
        fingerprint: Option<String>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut lease = self.lease(cancel).await?;
        let mut grants = decode(lease.read(cancel).await)?;
        match fingerprint {
            Some(value) => {
                grants.approved.insert(key, value);
            }
            None => {
                grants.approved.remove(&key);
            }
        }
        let bytes = serde_json::to_vec(&grants)
            .map_err(|_| SettingsError::Invalid("cannot encode MCP approvals"))?;
        lease.replace(bytes, private_permissions(), cancel).await?;
        self.changed.update(|version| *version += 1);
        Ok(())
    }
    async fn lease(&self, cancel: &CancellationToken) -> Result<FileLease> {
        let path = self.files.path(self.home_root, FILE)?;
        loop {
            match self.files.lock(&path, cancel).await {
                Ok(lease) => return Ok(lease),
                Err(FileStorageError::Locked) => tokio::select! {
                    _ = self.stopping.cancelled() => return Err(SettingsError::Closed),
                    _ = cancel.cancelled() => return Err(SettingsError::Cancelled),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {},
                },
                Err(error) => return Err(error.into()),
            }
        }
    }
}
