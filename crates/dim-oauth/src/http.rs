//! Account-service endpoints: discovery, token exchange, refresh and userinfo.
use crate::{Identity, Tokens, now};
use dim_app_server::managed_provider::ManagedError;
use regex::Regex;
use reqwest::{Client, RequestBuilder, StatusCode};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, sync::LazyLock};
use tokio_util::sync::CancellationToken;
use url::Url;

const SCOPE: &str = "openid profile email remote:delegate";
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

type Result<T> = std::result::Result<T, ManagedError>;
pub(super) type Body = Map<String, Value>;

/// Endpoints for one login, discovered from the selected directory.
#[derive(Clone)]
pub(super) struct Config {
    pub issuer: String,
    pub client_id: String,
    pub scope: String,
    pub authorization: String,
    pub token: String,
    pub device: String,
    pub device_token: String,
    pub relay: String,
}

/// The public OAuth client IDs of the account directories.
#[derive(Clone, Debug)]
pub struct ClientIds {
    /// For the `dimagent.com` directory.
    pub global: String,
    /// For every other directory.
    pub cn: String,
}

pub(super) fn identity_headers() -> HashMap<String, String> {
    HashMap::from([
        (
            "User-Agent".into(),
            format!("DimCode/{}", env!("CARGO_PKG_VERSION")),
        ),
        ("X-Title".into(), "DimCode".into()),
        ("HTTP-Referer".into(), "https://dimagent.com/".into()),
    ])
}

/// A non-blank string, returned as given.
pub(super) fn text(value: Option<&Value>, code: ManagedError) -> Result<String> {
    match value {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        _ => Err(code),
    }
}
pub(super) fn optional_text(value: Option<&Value>) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        value => text(value, ManagedError::InvalidResponse).map(Some),
    }
}
/// An absolute HTTP(S) URL without credentials or fragment.
pub(super) fn http_url(value: Option<&Value>, code: ManagedError) -> Result<Url> {
    let url = Url::parse(&text(value, code)?).map_err(|_| code)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some_and(|f| !f.is_empty())
    {
        return Err(code);
    }
    Ok(url)
}
pub(super) fn trimmed(url: &str) -> &str {
    url.trim_end_matches('/')
}
/// JavaScript's `Number.isSafeInteger` for non-negative values.
pub(super) fn safe_integer(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(n) => n.as_u64().or_else(|| {
            n.as_f64()
                .filter(|f| *f >= 0. && f.fract() == 0.)
                .map(|f| f as u64)
        }),
        _ => None,
    }
    .filter(|n| *n <= MAX_SAFE_INTEGER)
}

async fn cancellable<T>(
    future: impl Future<Output = T>,
    cancel: Option<&CancellationToken>,
) -> Option<T> {
    match cancel {
        Some(cancel) => tokio::select! {
            _ = cancel.cancelled() => None,
            value = future => Some(value),
        },
        None => Some(future.await),
    }
}

/// Sends one request. Transport failures and transient statuses become
/// `Transient`; a malformed body only fails a successful response, so a
/// caller's controlled 401 retry still sees the status.
pub(super) async fn request(
    request: RequestBuilder,
    cancel: Option<&CancellationToken>,
) -> Result<(StatusCode, Body)> {
    let exchange = async {
        let mut response = request.send().await.ok()?;
        let status = response.status();
        if status.is_redirection()
            || matches!(status.as_u16(), 408 | 429)
            || status.is_server_error()
        {
            return Some(Err(ManagedError::Transient));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
                return Some(Err(ManagedError::InvalidResponse));
            }
            bytes.extend_from_slice(&chunk);
        }
        Some(Ok((status, bytes)))
    };
    let (status, bytes) = cancellable(exchange, cancel)
        .await
        .flatten()
        .ok_or(ManagedError::Transient)??;
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(body)) => Ok((status, body)),
        _ if status.is_success() => Err(ManagedError::InvalidResponse),
        _ => Ok((status, Map::new())),
    }
}

pub(super) fn rejected(status: StatusCode, body: &Body) -> ManagedError {
    if status == StatusCode::UNAUTHORIZED
        || matches!(
            body.get("error").and_then(Value::as_str),
            Some("invalid_grant" | "invalid_token")
        )
    {
        ManagedError::RefreshRejected
    } else {
        ManagedError::InvalidResponse
    }
}

/// `expires_in` (default one hour) as an absolute millisecond deadline.
pub(super) fn expiry(body: &Body) -> Result<u64> {
    static DECIMAL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^[-+]?(?:\d+\.?\d*|\.\d+)(?:e[-+]?\d+)?$").unwrap());
    let seconds = match body.get("expires_in") {
        None | Some(Value::Null) => 3600.,
        Some(Value::Number(n)) => n.as_f64().ok_or(ManagedError::InvalidResponse)?,
        Some(Value::String(s)) if DECIMAL.is_match(s.trim()) => s
            .trim()
            .trim_start_matches('+')
            .parse()
            .map_err(|_| ManagedError::InvalidResponse)?,
        _ => return Err(ManagedError::InvalidResponse),
    };
    let expires = now() as f64 + seconds * 1000.;
    if seconds < 0. || !expires.is_finite() || expires > MAX_SAFE_INTEGER as f64 {
        return Err(ManagedError::InvalidResponse);
    }
    Ok(expires.trunc() as u64)
}

pub(super) async fn discover(
    client: &Client,
    ids: &crate::ClientIds,
    directory: &str,
    email: Option<&str>,
) -> Result<Config> {
    let base = http_url(
        Some(&Value::String(directory.into())),
        ManagedError::ConfigError,
    )?;
    if base.query().is_some_and(|q| !q.is_empty()) {
        return Err(ManagedError::ConfigError);
    }
    static EMAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^\s@]+@[^\s@]+$").unwrap());
    let email = email.map(str::trim);
    if email.is_some_and(|email| !EMAIL.is_match(email)) {
        return Err(ManagedError::ConfigError);
    }
    let mut issuer = trimmed(base.as_str()).to_owned();
    let mut client_id = if base.host_str() == Some("dimagent.com") {
        ids.global.clone()
    } else {
        ids.cn.clone()
    };
    if let Some(email) = email.filter(|email| !email.is_empty()) {
        // Realm discovery falls back to the selected public directory.
        let realm = async {
            let mut url = Url::parse(&format!("{issuer}/api/realm/discover")).ok()?;
            url.query_pairs_mut().append_pair("email", email);
            let (status, body) = request(client.get(url), None).await.ok()?;
            if !status.is_success() || !body.get("is_default").is_some_and(Value::is_boolean) {
                return None;
            }
            let endpoint = http_url(body.get("endpoint"), ManagedError::ConfigError).ok()?;
            let configured = optional_text(body.get("client_id")).ok()?;
            Some((trimmed(endpoint.as_str()).to_owned(), configured))
        };
        if let Some((endpoint, configured)) = realm.await {
            issuer = endpoint;
            if let Some(configured) = configured {
                client_id = configured;
            }
        }
    }
    let meta = match request(
        client.get(format!("{issuer}/.well-known/openid-configuration")),
        None,
    )
    .await
    {
        Ok((status, body)) if status.is_success() => body,
        _ => Map::new(),
    };
    let endpoint = |key: &str, fallback: &str| {
        http_url(meta.get(key), ManagedError::ConfigError)
            .map(String::from)
            .unwrap_or_else(|_| format!("{issuer}/{fallback}"))
    };
    let token = endpoint("token_endpoint", "oauth/token");
    let device_token = match meta.get("device_token_endpoint") {
        Some(value) => http_url(Some(value), ManagedError::ConfigError)
            .map(String::from)
            .unwrap_or_else(|_| token.clone()),
        None => token.clone(),
    };
    Ok(Config {
        authorization: endpoint("authorization_endpoint", "oauth/authorize"),
        device: endpoint(
            "device_authorization_endpoint",
            "oauth/device_authorization",
        ),
        relay: format!("{issuer}/v1"),
        issuer,
        client_id,
        scope: SCOPE.into(),
        token,
        device_token,
    })
}

fn bearer_type(body: &Body) -> Result<()> {
    if text(body.get("token_type"), ManagedError::InvalidResponse)?.to_lowercase() != "bearer" {
        return Err(ManagedError::InvalidResponse);
    }
    Ok(())
}

pub(super) fn token_from(config: &Config, body: &Body) -> Result<Tokens> {
    bearer_type(body)?;
    let scope = optional_text(body.get("scope"))?.unwrap_or_else(|| config.scope.clone());
    let tokens = json!({
        "type": "oauth",
        "access": text(body.get("access_token"), ManagedError::InvalidResponse)?,
        "refresh": text(body.get("refresh_token"), ManagedError::InvalidResponse)?,
        "expires": expiry(body)?,
        "issuer": config.issuer,
        "tokenEndpoint": config.token,
        "authorizationEndpoint": config.authorization,
        "deviceAuthorizationEndpoint": config.device,
        "deviceTokenEndpoint": config.device_token,
        "clientId": config.client_id,
        "scope": scope,
        "relayBaseUrl": config.relay,
    });
    let Value::Object(tokens) = tokens else {
        unreachable!("token record is an object")
    };
    Ok(Tokens(tokens))
}

pub(super) async fn refresh(client: &Client, tokens: &Tokens) -> Result<Tokens> {
    let (status, body) = request(
        client.post(tokens.str("tokenEndpoint")).form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", tokens.str("refresh")),
            ("client_id", tokens.str("clientId")),
        ]),
        None,
    )
    .await?;
    if !status.is_success() {
        return Err(rejected(status, &body));
    }
    bearer_type(&body)?;
    let mut rotated = tokens.clone();
    let fields = &mut rotated.0;
    fields.insert(
        "access".into(),
        text(body.get("access_token"), ManagedError::InvalidResponse)?.into(),
    );
    if let Some(refresh) = optional_text(body.get("refresh_token"))? {
        fields.insert("refresh".into(), refresh.into());
    }
    fields.insert("expires".into(), expiry(&body)?.into());
    if let Some(scope) = optional_text(body.get("scope"))? {
        fields.insert("scope".into(), scope.into());
    }
    Ok(rotated)
}

pub(super) async fn userinfo(
    client: &Client,
    tokens: &Tokens,
    cancel: Option<&CancellationToken>,
) -> Result<Identity> {
    let (status, body) = request(
        client
            .get(format!("{}/oauth/userinfo", trimmed(tokens.str("issuer"))))
            .bearer_auth(tokens.str("access")),
        cancel,
    )
    .await?;
    if matches!(status.as_u16(), 401 | 403) {
        return Err(ManagedError::IdentityRejected);
    }
    if !status.is_success() {
        return Err(ManagedError::InvalidResponse);
    }
    let label = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    Ok(Identity {
        issuer: tokens.str("issuer").to_owned(),
        subject: text(body.get("sub"), ManagedError::InvalidResponse)?,
        email: label("email"),
        name: label("name"),
    })
}
