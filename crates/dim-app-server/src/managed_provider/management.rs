//! Managed provider account wire boundary.
use super::ManagedProviders;
use crate::{
    methods::ManagementMethods,
    protocol::{Empty, optional_non_null},
    provider::management::ProviderTarget,
};
use dim_agent_capabilities::methods::{Interruption, MethodDecl, RpcError, params, reply};
use rutis::{Ctx, Error};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginBegin {
    pub provider_id: String,
    pub directory: String,
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub email: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserLoginBegin {
    pub provider_id: String,
    pub directory: String,
    #[serde(
        default,
        deserialize_with = "optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub email: Option<String>,
    pub redirect_uri: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedBrowserLoginComplete {
    pub provider_id: String,
    pub attempt_id: String,
    pub callback_url: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedLoginAttempt {
    pub provider_id: String,
    pub attempt_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginAttempt {
    pub attempt_id: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserLogin {
    pub attempt_id: String,
    pub authorization_url: String,
    pub expires_at: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLogin {
    pub attempt_id: String,
    pub verification_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub user_code: String,
    pub expires_at: u64,
}
/// The account's plan and consumption, as the plugin projects the account
/// service's usage report. Every part is optional: a plan without credits,
/// an unlimited bucket (its totals read 0) and a feature meter without an
/// allowance are all valid.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<UsagePlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<UsageCredits>,
    #[serde(default)]
    pub windows: Vec<UsageWindow>,
    #[serde(default)]
    pub features: Vec<UsageFeature>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsagePlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_amount: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_interval: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageCredits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_units: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_units: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_units: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unlimited: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageWindow {
    pub duration_hours: u64,
    pub token_used: u64,
    pub token_cap: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageFeature {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unlimited: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_end: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum ManagedLoginState {
    Pending,
    Completed,
    Failed { error: String },
    Cancelled,
    Expired,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum LoginState {
    Pending,
    Completed {
        account: crate::managed_provider::AccountSnapshot,
    },
    Failed {
        error: String,
    },
    Cancelled,
    Expired,
}

pub(super) fn provide(
    ctx: &Ctx,
    methods: &ManagementMethods,
    oauth: Arc<ManagedProviders>,
) -> Result<(), Error> {
    methods.register(
        ctx,
        "account",
        [
            "account.providers",
            "account.status",
            "account.recover",
            "account.login.begin",
            "account.login.browser.begin",
            "account.login.browser.complete",
            "account.login.status",
            "account.login.cancel",
            "account.logout",
            "account.usage",
        ]
        .into_iter()
        .map(|name| {
            MethodDecl::new(
                name,
                !matches!(
                    name,
                    "account.providers"
                        | "account.status"
                        | "account.login.status"
                        | "account.usage"
                ),
                Interruption::Drain,
            )
        })
        .collect(),
        Arc::new(move |method, value, stopping, _| {
            let oauth = oauth.clone();
            Box::pin(async move {
                let cancel = &stopping;
                match method.as_str() {
                    "account.providers" => {
                        let _: Empty = params(value)?;
                        reply(oauth.providers(cancel).await.map_err(error)?)
                    }
                    "account.status" => {
                        let p: ProviderTarget = params(value)?;
                        reply(oauth.status(&p.provider_id, cancel).await.map_err(error)?)
                    }
                    "account.recover" => {
                        let p: ProviderTarget = params(value)?;
                        reply(
                            oauth
                                .recover_account(&p.provider_id, cancel)
                                .await
                                .map_err(error)?,
                        )
                    }
                    "account.login.begin" => {
                        let p: LoginBegin = params(value)?;
                        reply(oauth.begin_device(p, cancel).await.map_err(error)?)
                    }
                    "account.login.browser.begin" => {
                        let p: BrowserLoginBegin = params(value)?;
                        reply(oauth.begin_browser(p, cancel).await.map_err(error)?)
                    }
                    "account.login.browser.complete" => {
                        let p: ManagedBrowserLoginComplete = params(value)?;
                        reply(oauth.complete_browser(p, cancel).await.map_err(error)?)
                    }
                    "account.login.status" => {
                        let p: ManagedLoginAttempt = params(value)?;
                        reply(match oauth.attempt_status(&p, cancel).map_err(error)? {
                            LoginState::Failed { .. } => LoginState::Failed {
                                error: "Login failed".into(),
                            },
                            status => status,
                        })
                    }
                    "account.login.cancel" => {
                        let p: ManagedLoginAttempt = params(value)?;
                        oauth.cancel_attempt(p, cancel).await.map_err(error)?;
                        reply(Empty {})
                    }
                    "account.logout" => {
                        let p: ProviderTarget = params(value)?;
                        reply(oauth.logout(&p.provider_id, cancel).await.map_err(error)?)
                    }
                    "account.usage" => {
                        let p: ProviderTarget = params(value)?;
                        reply(oauth.usage(&p.provider_id, cancel).await.map_err(error)?)
                    }
                    _ => unreachable!("registered account method"),
                }
            })
        }),
    )?;
    Ok(())
}

fn error(error: super::ManagedProviderError) -> RpcError {
    RpcError::app(-32000, error.code, error.message)
}

// Shared outbound boundary for the two built-in OAuth owners, not business logic.
pub(crate) fn attempt_id(params: &LoginAttempt) -> Result<uuid::Uuid, RpcError> {
    uuid::Uuid::parse_str(&params.attempt_id)
        .map_err(|_| RpcError::invalid("attemptId must be a UUID"))
}
pub(crate) fn managed_attempt(status: crate::managed_oauth::AttemptStatus) -> ManagedLoginState {
    use crate::managed_oauth::AttemptStatus;
    match status {
        AttemptStatus::Pending => ManagedLoginState::Pending,
        AttemptStatus::Completed => ManagedLoginState::Completed,
        AttemptStatus::Failed(_) => ManagedLoginState::Failed {
            error: "oauth_failed".into(),
        },
        AttemptStatus::Cancelled => ManagedLoginState::Cancelled,
        AttemptStatus::Expired => ManagedLoginState::Expired,
    }
}
pub(crate) fn managed_oauth_error(error: crate::managed_oauth::OAuthError) -> RpcError {
    use crate::managed_oauth::OAuthError;
    let kind = match error {
        OAuthError::AccountChanged => "account_changed",
        OAuthError::AttemptNotFound => "login_attempt_unknown",
        OAuthError::AttemptClosed => "login_attempt_closed",
        OAuthError::Missing | OAuthError::Rejected => "account_required",
        OAuthError::InvalidStore(_) => "account_invalid_store",
        OAuthError::UnsupportedSchema => "account_unsupported_schema",
        OAuthError::RecoveryRequired => "account_recovery_required",
        OAuthError::InvalidCallback(_) | OAuthError::Config(_) => "account_invalid_input",
        OAuthError::TooManyAttempts => "login_attempt_limit",
        OAuthError::Transient(_) => "account_unavailable",
        OAuthError::Closed => "service_reloading",
        _ => "account_failed",
    };
    let message = match error {
        OAuthError::Storage(_) | OAuthError::Worker(_) => "OAuth account operation failed".into(),
        error => error.to_string(),
    };
    RpcError::app(-32000, kind, message)
}
