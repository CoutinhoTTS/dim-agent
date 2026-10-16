//! OAuth token requests, which Mcp sends itself.
use super::{
    McpAuthError,
    store::{AuthMethod, Client, Tokens},
};
use crate::mcp::config;
use reqwest_mcp::{Client as Http, StatusCode, Url};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Every OAuth request: no redirects, no retries, the 30 s budget rmcp
/// also uses.
pub(super) fn http() -> Result<Http, McpAuthError> {
    Http::builder()
        .redirect(reqwest_mcp::redirect::Policy::none())
        .retry(reqwest_mcp::retry::never())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| McpAuthError::Unavailable)
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) fn loopback(url: &Url) -> bool {
    url.host().is_some_and(|host| config::loopback(&host))
}

async fn body(mut response: reqwest_mcp::Response) -> Result<Vec<u8>, McpAuthError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| McpAuthError::Unavailable)?
    {
        if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
            return Err(McpAuthError::Unavailable);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn split(scopes: &str) -> Vec<String> {
    scopes.split_whitespace().map(str::to_owned).collect()
}

pub(super) enum Grant<'a> {
    Refresh(&'a str),
}

pub(super) enum TokenError {
    /// `invalid_grant`: the code or refresh token is no longer valid.
    Rejected,
    /// `invalid_client`: the registration is gone.
    InvalidClient,
    /// Another refusal.
    Failed,
    /// Network failure, timeout, a redirect, 5xx, 429 or a response that is
    /// not an OAuth error: nothing is known to have changed.
    Unavailable,
}

/// One token request (RFC 6749 §4.1.3, §6) with the record's `resource`
/// (RFC 8707) and client authentication. Never logged.
pub(super) async fn token(
    http: &Http,
    endpoint: &str,
    client: &Client,
    resource: &str,
    grant: Grant<'_>,
    requested: &[String],
) -> Result<Tokens, TokenError> {
    let mut form: Vec<(&str, &str)> = match &grant {
        Grant::Refresh(refresh) => {
            vec![("grant_type", "refresh_token"), ("refresh_token", refresh)]
        }
    };
    form.push(("resource", resource));
    let request = authenticate(http.post(endpoint), &mut form, client);
    let response = form_body(request, &form)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| TokenError::Unavailable)?;
    let status = response.status();
    if status.is_server_error()
        || status.is_redirection()
        || matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS | StatusCode::REQUEST_TIMEOUT
        )
    {
        return Err(TokenError::Unavailable);
    }
    let bytes = body(response).await.map_err(|_| TokenError::Unavailable)?;
    if !status.is_success() {
        // Only an OAuth error response (RFC 6749 §5.2) is a refusal; anything
        // else says nothing about the grant.
        let error = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| value.get("error")?.as_str().map(str::to_owned));
        return Err(match error.as_deref() {
            Some("invalid_grant") => TokenError::Rejected,
            Some("invalid_client") => TokenError::InvalidClient,
            Some(_) => TokenError::Failed,
            None => TokenError::Unavailable,
        });
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| TokenError::Failed)?;
    // An empty optional field is absent, as many servers send it.
    let string = |name: &str| match value.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.is_empty() => Ok(None),
        Some(Value::String(value)) if !value.chars().any(char::is_control) => {
            Ok(Some(value.clone()))
        }
        _ => Err(TokenError::Failed),
    };
    let access_token = string("access_token")?.ok_or(TokenError::Failed)?;
    if string("token_type")?.is_some_and(|kind| !kind.eq_ignore_ascii_case("bearer")) {
        return Err(TokenError::Failed);
    }
    let expires_at = match value.get("expires_in") {
        None | Some(Value::Null) => None,
        Some(seconds) => Some(
            seconds
                .as_u64()
                .filter(|seconds| *seconds > 0)
                .and_then(|seconds| seconds.checked_mul(1000))
                .and_then(|ms| now().checked_add(ms))
                .ok_or(TokenError::Failed)?,
        ),
    };
    Ok(Tokens {
        access_token,
        refresh_token: string("refresh_token")?,
        expires_at,
        // RFC 6749 §5.1: an omitted scope means the request was granted.
        scopes: string("scope")?.map_or_else(|| requested.to_vec(), |scope| split(&scope)),
    })
}

fn form_body(
    request: reqwest_mcp::RequestBuilder,
    form: &[(&str, &str)],
) -> reqwest_mcp::RequestBuilder {
    request
        .header(
            reqwest_mcp::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(form)
                .finish(),
        )
}

/// RFC 6749 §2.3.1: Basic credentials are form-encoded before Base64.
fn authenticate<'a>(
    request: reqwest_mcp::RequestBuilder,
    form: &mut Vec<(&'a str, &'a str)>,
    client: &'a Client,
) -> reqwest_mcp::RequestBuilder {
    match (client.auth_method, client.client_secret.as_deref()) {
        (AuthMethod::ClientSecretBasic, Some(secret)) => {
            let encode = |value: &str| {
                url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>()
            };
            request.basic_auth(encode(&client.client_id), Some(encode(secret)))
        }
        (AuthMethod::ClientSecretPost, Some(secret)) => {
            form.push(("client_id", &client.client_id));
            form.push(("client_secret", secret));
            request
        }
        _ => {
            form.push(("client_id", &client.client_id));
            request
        }
    }
}
