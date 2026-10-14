//! Codex OAuth account wire boundary.
use super::OpenAICodexOAuthService;
use crate::{
    managed_provider::management::{attempt_id, managed_attempt, managed_oauth_error as error},
    methods::{Interruption, MethodDecl, Methods, params, reply},
    protocol::*,
};
use dim_agent_runtime::{Capability, KeysOf, scoped};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::sync::Arc;

pub struct CodexAccountManagement;
impl TypedPlugin for CodexAccountManagement {
    type Deps = (Arc<Methods>, Arc<OpenAICodexOAuthService>);
    fn name(&self) -> &str {
        "CodexAccountManagement"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (management, oauth): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let registration = management
                .register(
                    "account.codex",
                    [
                        "account.codex.status",
                        "account.codex.recover",
                        "account.codex.login.browser.begin",
                        "account.codex.login.browser.complete",
                        "account.codex.login.status",
                        "account.codex.login.cancel",
                        "account.codex.logout",
                    ]
                    .into_iter()
                    .map(|name| {
                        MethodDecl::management(
                            name,
                            !matches!(name, "account.codex.status" | "account.codex.login.status"),
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
                                "account.codex.status" => {
                                    let _: Empty = params(value)?;
                                    reply(oauth.status(cancel).await.map_err(error)?)
                                }
                                "account.codex.recover" => {
                                    let _: Empty = params(value)?;
                                    oauth.recover(cancel).await.map_err(error)?;
                                    reply(Empty {})
                                }
                                "account.codex.login.browser.begin" => {
                                    let _: Empty = params(value)?;
                                    let login = oauth.begin_browser(cancel).await.map_err(error)?;
                                    reply(BrowserLogin {
                                        attempt_id: login.attempt_id.to_string(),
                                        authorization_url: login.authorization_url.into(),
                                        expires_at: login.expires_at,
                                    })
                                }
                                "account.codex.login.browser.complete" => {
                                    let p: BrowserLoginComplete = params(value)?;
                                    let id =
                                        uuid::Uuid::parse_str(&p.attempt_id).map_err(|_| {
                                            RpcError::invalid("attemptId must be a UUID")
                                        })?;
                                    let callback =
                                        url::Url::parse(&p.callback_url).map_err(|_| {
                                            RpcError::invalid("callbackUrl must be a callback URL")
                                        })?;
                                    oauth
                                        .complete_browser(id, callback, cancel)
                                        .await
                                        .map_err(error)?;
                                    reply(Empty {})
                                }
                                "account.codex.login.status" => {
                                    let p: LoginAttempt = params(value)?;
                                    reply(managed_attempt(
                                        oauth
                                            .attempt_status(attempt_id(&p)?)
                                            .await
                                            .map_err(error)?,
                                    ))
                                }
                                "account.codex.login.cancel" => {
                                    let p: LoginAttempt = params(value)?;
                                    oauth.cancel_attempt(attempt_id(&p)?).await.map_err(error)?;
                                    reply(Empty {})
                                }
                                "account.codex.logout" => {
                                    let _: Empty = params(value)?;
                                    oauth.logout(cancel).await.map_err(error)?;
                                    reply(Empty {})
                                }
                                _ => unreachable!("registered Codex account method"),
                            }
                        })
                    }),
                )
                .map_err(|error| CordisError::PluginFailed(error.message.into()))?;
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        registration.unregister_and_drain().await;
                        Ok(())
                    })
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for CodexAccountManagement {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}
