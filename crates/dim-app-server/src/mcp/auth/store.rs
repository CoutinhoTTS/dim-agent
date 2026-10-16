//! `mcp-oauth.json` under the home root: Mcp's own credential store. One
//! writer (Mcp), atomic 0600 replace through FileStorage, versioned.
use super::{CredentialKey, McpAuthError};
use dim_agent_capabilities::file_storage::{FileLease, FilePath, FileStorage, FileStorageError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{io, sync::Arc, time::Duration};
use tokio::sync::{Mutex, MutexGuard};
use tokio_util::sync::CancellationToken;

const FILE: &str = "mcp-oauth.json";
const VERSION: u64 = 1;

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Document {
    pub credentials: Vec<Record>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}
impl Document {
    pub fn get(&self, key: &CredentialKey) -> Option<&Record> {
        self.credentials.iter().find(|record| record.is(key))
    }
    pub fn get_mut(&mut self, key: &CredentialKey) -> Option<&mut Record> {
        self.credentials.iter_mut().find(|record| record.is(key))
    }
}

/// One credential key's record. A record without `tokens` is expired: the
/// user must sign in again. Secrets never leave this module except as
/// request material; nothing here implements `Debug`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Record {
    pub server_id: String,
    pub url: String,
    /// The `resource` sent on the authorization request, repeated on every
    /// token request.
    pub resource: String,
    pub issuer: String,
    pub token_endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_endpoint: Option<String>,
    /// Cleared when the authorization server answers `invalid_client`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<Client>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Tokens>,
    pub requested_scopes: Vec<String>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}
impl Record {
    fn is(&self, key: &CredentialKey) -> bool {
        self.server_id == key.server_id && self.url == key.url
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Client {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub registration: Registration,
    pub redirect_uri: String,
    pub auth_method: AuthMethod,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Registration {
    Preregistered,
    Dynamic,
}

/// Client authentication at the token and revocation endpoints.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AuthMethod {
    None,
    ClientSecretBasic,
    ClientSecretPost,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Tokens {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix ms, computed from `expires_in` on receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    pub scopes: Vec<String>,
}

pub(super) struct Store {
    files: Arc<FileStorage>,
    path: FilePath,
    /// In front of the FileStorage lock, which admits or fails at once.
    local: Mutex<()>,
}

/// The store held for one read-modify-write, including any token request
/// made in between.
pub(super) struct Locked<'a> {
    _local: MutexGuard<'a, ()>,
    lease: FileLease,
    pub document: Document,
}
impl Locked<'_> {
    /// Commits once started: the replace ignores cancellation.
    pub async fn write(&mut self) -> Result<(), McpAuthError> {
        let mut value =
            serde_json::to_value(&self.document).map_err(|_| McpAuthError::Unavailable)?;
        value
            .as_object_mut()
            .ok_or(McpAuthError::Unavailable)?
            .insert("version".into(), VERSION.into());
        let mut bytes = serde_json::to_vec_pretty(&value).map_err(|_| McpAuthError::Unavailable)?;
        bytes.push(b'\n');
        #[cfg(unix)]
        let permissions = Some(std::os::unix::fs::PermissionsExt::from_mode(0o600));
        #[cfg(not(unix))]
        let permissions = None;
        self.lease
            .replace(bytes, permissions, &CancellationToken::new())
            .await
            .map_err(|_| McpAuthError::Unavailable)
    }
}

impl Store {
    pub fn new(files: Arc<FileStorage>, root: usize) -> Result<Self, FileStorageError> {
        Ok(Self {
            path: files.path(root, FILE)?,
            files,
            local: Mutex::new(()),
        })
    }

    /// A lock-free read: writes are atomic replaces.
    pub async fn read(&self, cancel: &CancellationToken) -> Result<Document, McpAuthError> {
        decode(self.files.read(&self.path, cancel).await)
    }

    /// Another daemon sharing the home may hold the file lock; retry until
    /// it is released or the caller stops.
    pub async fn lock(&self, cancel: &CancellationToken) -> Result<Locked<'_>, McpAuthError> {
        let local = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(McpAuthError::Cancelled),
            guard = self.local.lock() => guard,
        };
        let mut lease = loop {
            match self.files.lock(&self.path, cancel).await {
                Ok(lease) => break lease,
                Err(FileStorageError::Locked) => tokio::select! {
                    _ = cancel.cancelled() => return Err(McpAuthError::Cancelled),
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {}
                },
                Err(FileStorageError::Cancelled) => return Err(McpAuthError::Cancelled),
                Err(_) => return Err(McpAuthError::Unavailable),
            }
        };
        let document = decode(lease.read(cancel).await)?;
        Ok(Locked {
            _local: local,
            lease,
            document,
        })
    }
}

/// An unknown version or an unreadable document is never overwritten.
fn decode(read: Result<Vec<u8>, FileStorageError>) -> Result<Document, McpAuthError> {
    let bytes = match read {
        Ok(bytes) => bytes,
        Err(FileStorageError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Document::default());
        }
        Err(FileStorageError::Cancelled) => return Err(McpAuthError::Cancelled),
        Err(_) => return Err(McpAuthError::Unavailable),
    };
    let mut value: Value = serde_json::from_slice(&bytes).map_err(|_| McpAuthError::Unsupported)?;
    let object = value.as_object_mut().ok_or(McpAuthError::Unsupported)?;
    if object.remove("version").and_then(|v| v.as_u64()) != Some(VERSION) {
        return Err(McpAuthError::Unsupported);
    }
    serde_json::from_value(value).map_err(|_| McpAuthError::Unsupported)
}
