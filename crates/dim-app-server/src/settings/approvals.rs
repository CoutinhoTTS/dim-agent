//! Host-owned grants: business owners supply scoped identity and fingerprint digests.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grants {
    version: u32,
    approved: BTreeMap<String, String>,
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
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
        .map_err(|_| SettingsError::Invalid("invalid approval store"))?;
    if grants.version != 1
        || grants
            .approved
            .iter()
            .any(|(key, value)| !digest(key) || !digest(value))
    {
        return Err(SettingsError::Invalid("invalid approval store"));
    }
    Ok(grants)
}
impl SettingsService {
    pub async fn approved(
        &self,
        namespace: &str,
        key: &str,
        fingerprint: &str,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        self.check_open(cancel)?;
        validate_namespace(namespace)?;
        if !digest(key) || !digest(fingerprint) {
            return Err(SettingsError::Invalid("invalid approval digest"));
        }
        let path = self
            .files
            .path(self.home_root, format!("{namespace}-approvals.json"))?;
        let grants = decode_grants(self.files.read(&path, cancel).await)?;
        Ok(grants
            .approved
            .get(key)
            .is_some_and(|value| value == fingerprint))
    }
    pub async fn set_approval(
        self: &Arc<Self>,
        namespace: &str,
        key: String,
        fingerprint: Option<String>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        validate_namespace(namespace)?;
        if !digest(&key) || fingerprint.as_ref().is_some_and(|value| !digest(value)) {
            return Err(SettingsError::Invalid("invalid approval digest"));
        }
        let path = self
            .files
            .path(self.home_root, format!("{namespace}-approvals.json"))?;
        let task = {
            let _guard = self.gate.lock().unwrap();
            self.check_open(cancel)?;
            let owner = self.clone();
            let cancel = cancel.clone();
            self.updates.spawn(async move {
                let mut lease = owner.lease(&path, &cancel).await?;
                let mut grants = decode_grants(lease.read(&cancel).await)?;
                match fingerprint {
                    Some(value) => {
                        grants.approved.insert(key, value);
                    }
                    None => {
                        grants.approved.remove(&key);
                    }
                }
                let bytes = serde_json::to_vec(&grants)
                    .map_err(|_| SettingsError::Invalid("cannot encode approvals"))?;
                lease.replace(bytes, private_permissions(), &cancel).await?;
                owner.changed.update(|version| *version += 1);
                Ok(())
            })
        };
        task.await?
    }
}
