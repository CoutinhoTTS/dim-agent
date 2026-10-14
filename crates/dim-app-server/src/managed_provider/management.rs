//! Managed provider account wire boundary.
use super::{AccountSnapshot, ManagedProviders};
use crate::{
    methods::{Interruption, ManagementMethods, MethodDecl, params, reply},
    protocol::*,
};
use rutis::{CordisError, Ctx};
use std::sync::Arc;

pub(super) fn provide(ctx: &Ctx, oauth: Arc<ManagedProviders>) -> Result<(), CordisError> {
    ManagementMethods::provide(
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
        ]
        .into_iter()
        .map(|name| {
            MethodDecl::management(
                name,
                !matches!(
                    name,
                    "account.providers" | "account.status" | "account.login.status"
                ),
                Interruption::Drain,
            )
        })
        .collect(),
        vec![],
        Arc::new(move |method, value, stopping| {
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
                                .call::<AccountSnapshot>("recover", reply(p)?, cancel)
                                .await
                                .map_err(error)?,
                        )
                    }
                    "account.login.begin" => {
                        let p: LoginBegin = params(value)?;
                        reply(
                            oauth
                                .call::<DeviceLogin>("beginDevice", reply(p)?, cancel)
                                .await
                                .map_err(error)?,
                        )
                    }
                    "account.login.browser.begin" => {
                        let p: BrowserLoginBegin = params(value)?;
                        reply(
                            oauth
                                .call::<BrowserLogin>("beginBrowser", reply(p)?, cancel)
                                .await
                                .map_err(error)?,
                        )
                    }
                    "account.login.browser.complete" => {
                        let p: ManagedBrowserLoginComplete = params(value)?;
                        reply(
                            oauth
                                .call::<AccountSnapshot>("completeBrowser", reply(p)?, cancel)
                                .await
                                .map_err(error)?,
                        )
                    }
                    "account.login.status" => {
                        let p: ManagedLoginAttempt = params(value)?;
                        let status: LoginState = oauth
                            .call("attemptStatus", reply(p)?, cancel)
                            .await
                            .map_err(error)?;
                        reply(match status {
                            LoginState::Failed { .. } => LoginState::Failed {
                                error: "Login failed".into(),
                            },
                            status => status,
                        })
                    }
                    "account.login.cancel" => {
                        let p: ManagedLoginAttempt = params(value)?;
                        let _: serde_json::Value = oauth
                            .call("cancelAttempt", reply(p)?, cancel)
                            .await
                            .map_err(error)?;
                        reply(Empty {})
                    }
                    "account.logout" => {
                        let p: ProviderTarget = params(value)?;
                        reply(
                            oauth
                                .call::<AccountSnapshot>("logout", reply(p)?, cancel)
                                .await
                                .map_err(error)?,
                        )
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
