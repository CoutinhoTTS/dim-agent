pub(crate) mod management;
use crate::managed_oauth::{self, Service};
use dim_agent_capabilities::file_storage::FileStorage;
use dim_agent_runtime::{Capability, KeysOf, scoped};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypeKey, TypedPlugin};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

pub use managed_oauth::{
    AttemptStatus, AuthStatus, BrowserBegin, Config as OpenAICodexOAuthConfig, CredentialLease,
    OAuthError,
};
pub type Result<T> = managed_oauth::Result<T>;

pub struct OpenAICodexOAuthService(Arc<Service>);
impl OpenAICodexOAuthService {
    pub async fn status(&self, c: &CancellationToken) -> Result<AuthStatus> {
        self.0.status(c).await
    }
    pub async fn begin_browser(&self, c: &CancellationToken) -> Result<BrowserBegin> {
        self.0.begin_browser(c).await
    }
    pub async fn complete_browser(
        &self,
        id: Uuid,
        callback: Url,
        c: &CancellationToken,
    ) -> Result<()> {
        self.0.complete_browser(id, callback, c).await
    }
    pub async fn attempt_status(&self, id: Uuid) -> Result<AttemptStatus> {
        self.0.attempt_status(id).await
    }
    pub async fn cancel_attempt(&self, id: Uuid) -> Result<()> {
        self.0.cancel_attempt(id).await
    }
    pub async fn logout(&self, c: &CancellationToken) -> Result<()> {
        self.0.logout(c).await
    }
    pub async fn recover(&self, c: &CancellationToken) -> Result<()> {
        self.0.recover(c).await
    }
    pub async fn credential(
        &self,
        rejected: Option<&CredentialLease>,
        c: &CancellationToken,
    ) -> Result<CredentialLease> {
        self.0.credential(rejected, c).await
    }
    pub async fn finish_rejected(
        &self,
        rejected: &CredentialLease,
        c: &CancellationToken,
    ) -> Result<()> {
        self.0.finish_rejected(rejected, c).await
    }
}

pub struct OpenAICodexOAuthPlugin {
    root: usize,
    config: OpenAICodexOAuthConfig,
}
impl OpenAICodexOAuthPlugin {
    pub fn new(root: usize) -> Self {
        Self::with_config(root, production())
    }
    pub fn with_config(root: usize, config: OpenAICodexOAuthConfig) -> Self {
        Self { root, config }
    }
}
pub(crate) fn production() -> OpenAICodexOAuthConfig {
    OpenAICodexOAuthConfig {
        issuer: Url::parse("https://auth.openai.com").unwrap(),
        client_id: "app_EMoamEEZ73f0CkXaXp7hrann".into(),
        scope: "openid profile email offline_access api.connectors.read api.connectors.invoke"
            .into(),
        authorization: Some(Url::parse("https://auth.openai.com/oauth/authorize").unwrap()),
        token: Url::parse("https://auth.openai.com/oauth/token").unwrap(),
        device: None,
        redirect: Some(Url::parse("http://localhost:1455/auth/callback").unwrap()),
        relay: Url::parse("https://chatgpt.com/backend-api/codex").unwrap(),
    }
}
impl TypedPlugin for OpenAICodexOAuthPlugin {
    type Deps = (Arc<FileStorage>,);
    fn name(&self) -> &str {
        "OpenAICodexOAuth"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (files,): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Service::new(
                files,
                self.root,
                "openaiCodex",
                self.config.clone(),
                &production(),
            )
            .map_err(|e| CordisError::PluginFailed(e.into()))?;
            let closing = service.clone();
            if let Err(error) = ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        closing.close().await;
                        Ok(())
                    })
                }))
            }) {
                service.close().await;
                return Err(error);
            }
            ctx.provide_as(
                TypeKey::of::<OpenAICodexOAuthService>(),
                Arc::new(OpenAICodexOAuthService(service)),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for OpenAICodexOAuthPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}
