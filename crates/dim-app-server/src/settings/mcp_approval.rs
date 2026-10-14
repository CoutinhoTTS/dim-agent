//! Host-owned execution grants, separate from project-controlled configuration.
//! The private home file is shared by native and standalone settings hosts; it
//! contains only scoped digests, never commands, environment values or tokens.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grants {
    version: u32,
    approved: BTreeMap<String, String>,
}

fn decode_grants(bytes: std::result::Result<Vec<u8>, FileStorageError>) -> Result<Grants> {
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

impl SettingsService {
    pub(crate) fn mcp_workspace(&self, source: SettingsSource) -> Result<PathBuf> {
        let SettingsSource::ProjectMcp(root) = source else {
            return Err(SettingsError::Invalid(
                "MCP approval requires a project source",
            ));
        };
        self.projects
            .lock()
            .unwrap()
            .iter()
            .find(|(_, index)| **index == root)
            .map(|(path, _)| path.clone())
            .ok_or(SettingsError::Invalid("unauthorized project"))
    }

    pub(crate) fn mcp_effective(
        &self,
        source: SettingsSource,
        mut config: McpServerSettings,
    ) -> Result<McpServerSettings> {
        if matches!(source, SettingsSource::ProjectMcp(_)) {
            let workspace = self.mcp_workspace(source)?;
            if let McpServerSettings::Stdio { cwd, .. } = &mut config {
                // Resolve relative directories against the workspace, not the daemon.
                let path = cwd
                    .as_ref()
                    .map_or(workspace.clone(), |cwd| workspace.join(cwd));
                *cwd = Some(
                    path.to_str()
                        .ok_or(SettingsError::Invalid("project cwd must be UTF-8"))?
                        .to_owned(),
                );
            }
        }
        Ok(config)
    }

    fn mcp_grant_key(&self, source: SettingsSource, id: &str) -> Result<String> {
        let workspace = self.mcp_workspace(source)?;
        let bytes = serde_json::to_vec(&(workspace, id))
            .map_err(|_| SettingsError::Invalid("invalid MCP identity"))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub(crate) fn mcp_fingerprint(
        &self,
        source: SettingsSource,
        id: &str,
        config: &McpServerSettings,
    ) -> Result<String> {
        let key = self.mcp_grant_key(source, id)?;
        let mut config = config.clone();
        // Enablement is intent, not a new executable configuration.
        let (McpServerSettings::Stdio { enabled, .. } | McpServerSettings::Remote { enabled, .. }) =
            &mut config;
        *enabled = None;
        let bytes = serde_json::to_vec(&("dim/mcp-execution/v1", key, config))
            .map_err(|_| SettingsError::Invalid("invalid MCP configuration"))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub(crate) async fn mcp_approved(
        &self,
        source: SettingsSource,
        id: &str,
        fingerprint: &str,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        self.check_open(cancel)?;
        let key = self.mcp_grant_key(source, id)?;
        let path = self.files.path(self.home_root, "mcp-approvals.json")?;
        let grants = decode_grants(self.files.read(&path, cancel).await)?;
        Ok(grants
            .approved
            .get(&key)
            .is_some_and(|value| value == fingerprint))
    }

    pub(crate) async fn mcp_set_approval(
        self: &Arc<Self>,
        source: SettingsSource,
        id: String,
        fingerprint: Option<String>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let key = self.mcp_grant_key(source, &id)?;
        let path = self.files.path(self.home_root, "mcp-approvals.json")?;
        let task = {
            let _guard = self.gate.lock().unwrap();
            self.check_open(cancel)?;
            let owner = self.clone();
            let cancel = cancel.clone();
            self.updates.spawn(async move {
                let mut lease = loop {
                    owner.check_open(&cancel)?;
                    match owner.files.lock(&path, &cancel).await {
                        Ok(lease) => break lease,
                        Err(FileStorageError::Locked) => tokio::select! {
                            _ = owner.stopping.cancelled() => return Err(SettingsError::Closed),
                            _ = cancel.cancelled() => return Err(SettingsError::Cancelled),
                            _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {},
                        },
                        Err(error) => return Err(error.into()),
                    }
                };
                let mut grants = decode_grants(lease.read(&cancel).await)?;
                match fingerprint {
                    Some(fingerprint) => {
                        grants.approved.insert(key, fingerprint);
                    }
                    None => {
                        grants.approved.remove(&key);
                    }
                }
                let bytes = serde_json::to_vec(&grants)
                    .map_err(|_| SettingsError::Invalid("cannot encode MCP approvals"))?;
                #[cfg(unix)]
                let permissions = {
                    use std::os::unix::fs::PermissionsExt;
                    Some(std::fs::Permissions::from_mode(0o600))
                };
                #[cfg(not(unix))]
                let permissions = None;
                lease.replace(bytes, permissions, &cancel).await?;
                owner.changed.update(|version| *version += 1);
                Ok(())
            })
        };
        task.await?
    }
}
