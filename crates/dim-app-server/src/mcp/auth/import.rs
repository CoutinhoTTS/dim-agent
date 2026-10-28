use super::{
    CredentialKey, McpAuth, McpAuthError, oauth,
    store::{AuthMethod, Binding, Client, Record, Registration, Tokens},
};
use crate::mcp::config::McpOAuthSettings;
use dim_agent_types::AccountImportStatus;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Legacy {
    server_url: String,
    tokens: LegacyTokens,
    client_info: LegacyClient,
    authorization_metadata: Value,
    protected_resource_metadata: Option<Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<u64>,
    scope: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyClient {
    client_id: String,
    client_secret: Option<String>,
    client_id_issued_at: Option<u64>,
    client_secret_expires_at: Option<u64>,
    token_endpoint_auth_method: Option<String>,
    metadata: Option<Value>,
}

impl McpAuth {
    pub(crate) async fn import_legacy(
        self: &Arc<Self>,
        key: CredentialKey,
        configuration: Option<McpOAuthSettings>,
        entry: Value,
        cancel: &CancellationToken,
    ) -> Result<AccountImportStatus, McpAuthError> {
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&entry).map_err(|_| McpAuthError::Unsupported)?)
        );
        let legacy: Legacy =
            serde_json::from_value(entry).map_err(|_| McpAuthError::Unsupported)?;
        let server = Url::parse(&legacy.server_url).map_err(|_| McpAuthError::Unsupported)?;
        if CredentialKey::new(&key.server_id, &server) != key
            || !oauth::trusted(&server, oauth::loopback(&server))
        {
            return Err(McpAuthError::Unsupported);
        }
        let endpoint = |name: &str| -> Result<String, McpAuthError> {
            let raw = legacy.authorization_metadata[name]
                .as_str()
                .ok_or(McpAuthError::Unsupported)?;
            let parsed = Url::parse(raw).map_err(|_| McpAuthError::Unsupported)?;
            if !oauth::trusted(&parsed, oauth::loopback(&server)) {
                return Err(McpAuthError::Unsupported);
            }
            Ok(raw.into())
        };
        let issuer = endpoint("issuer")?;
        let token_endpoint = endpoint("token_endpoint")?;
        let resource = legacy
            .protected_resource_metadata
            .as_ref()
            .and_then(|value| value.get("resource"))
            .and_then(Value::as_str)
            .unwrap_or(&key.url)
            .to_owned();
        let resource_url = Url::parse(&resource).map_err(|_| McpAuthError::Unsupported)?;
        if resource_url.origin() != server.origin()
            || !oauth::trusted(&resource_url, oauth::loopback(&server))
        {
            return Err(McpAuthError::Unsupported);
        }
        let text = |value: &str| !value.trim().is_empty() && !value.chars().any(char::is_control);
        if !text(&legacy.client_info.client_id)
            || !text(&legacy.tokens.access_token)
            || legacy
                .tokens
                .refresh_token
                .as_deref()
                .is_some_and(|value| !text(value))
            || legacy.tokens.expires_at.is_some_and(|expires| {
                expires > dim_agent_types::JS_MAX_SAFE_INTEGER
                    || (expires <= oauth::now() && legacy.tokens.refresh_token.is_none())
            })
        {
            return Err(McpAuthError::Unsupported);
        }
        let registration = if let Some(client_id) = configuration
            .as_ref()
            .and_then(|configuration| configuration.client_id.as_deref())
        {
            if client_id != legacy.client_info.client_id {
                return Err(McpAuthError::Unsupported);
            }
            Registration::Preregistered
        } else {
            Registration::Dynamic
        };
        if legacy
            .client_info
            .client_id_issued_at
            .is_some_and(|at| at > dim_agent_types::JS_MAX_SAFE_INTEGER)
            || legacy
                .client_info
                .client_secret_expires_at
                .is_some_and(|at| {
                    at > dim_agent_types::JS_MAX_SAFE_INTEGER
                        || (at > 0 && at.saturating_mul(1000) <= oauth::now())
                })
        {
            return Err(McpAuthError::Unsupported);
        }
        let redirect_uri = legacy
            .client_info
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("redirectUri"))
            .and_then(Value::as_str)
            .ok_or(McpAuthError::Unsupported)?
            .to_owned();
        let redirect = Url::parse(&redirect_uri).map_err(|_| McpAuthError::Unsupported)?;
        if !oauth::loopback(&redirect)
            || redirect.scheme() != "http"
            || !redirect.username().is_empty()
            || redirect.password().is_some()
        {
            return Err(McpAuthError::Unsupported);
        }
        let auth_method = match legacy.client_info.token_endpoint_auth_method.as_deref() {
            None | Some("none") if legacy.client_info.client_secret.is_none() => AuthMethod::None,
            Some("client_secret_basic")
                if legacy
                    .client_info
                    .client_secret
                    .as_deref()
                    .is_some_and(text) =>
            {
                AuthMethod::ClientSecretBasic
            }
            Some("client_secret_post")
                if legacy
                    .client_info
                    .client_secret
                    .as_deref()
                    .is_some_and(text) =>
            {
                AuthMethod::ClientSecretPost
            }
            _ => return Err(McpAuthError::Unsupported),
        };
        let scopes = legacy
            .tokens
            .scope
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let record = Record::new(
            &key,
            Binding {
                resource,
                issuer,
                token_endpoint,
                revocation_endpoint: None,
                requested_scopes: configuration
                    .as_ref()
                    .and_then(|config| config.scopes.as_deref())
                    .map(|scopes| scopes.split_whitespace().map(str::to_owned).collect())
                    .unwrap_or_else(|| scopes.clone()),
            },
            Client {
                client_id: legacy.client_info.client_id,
                client_secret: legacy.client_info.client_secret,
                registration,
                redirect_uri,
                auth_method,
            },
            Tokens {
                access_token: legacy.tokens.access_token,
                refresh_token: legacy.tokens.refresh_token,
                expires_at: legacy.tokens.expires_at,
                scopes,
            },
        );
        let identity = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&key.server_id, &key.url))
                    .map_err(|_| McpAuthError::Unsupported)?
            )
        );
        self.owned(cancel, move |auth| async move {
            let mut locked = auth.store.lock(&auth.stopping).await?;
            if let Some(previous) = locked.document.legacy_imports.get(&identity) {
                return Ok(if previous == &digest {
                    AccountImportStatus::AlreadyImported
                } else {
                    AccountImportStatus::Conflict
                });
            }
            if locked.document.get(&key).is_some() {
                return Ok(AccountImportStatus::Conflict);
            }
            locked.document.put(record);
            locked.document.legacy_imports.insert(identity, digest);
            locked.write().await?;
            auth.changed(&key, true);
            Ok(AccountImportStatus::Imported)
        })
        .await
    }
}
