pub(crate) mod management;
use crate::managed_oauth::{self, Service};
use dim_agent_capabilities::file_storage::FileStorage;
use rutis::{BoxFuture, CordisError, Ctx, Effect, Keyed, TypeKey, TypedPlugin};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

pub use managed_oauth::{
    AttemptStatus, AuthStatus, Config as GrokOAuthConfig, CredentialLease, DeviceBegin, OAuthError,
};
pub type Result<T> = managed_oauth::Result<T>;
pub struct GrokOAuthService(Arc<Service>);
impl GrokOAuthService {
    pub async fn status(&self, c: &CancellationToken) -> Result<AuthStatus> {
        self.0.status(c).await
    }
    pub async fn begin_device(&self, c: &CancellationToken) -> Result<DeviceBegin> {
        self.0.begin_device(c).await
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
pub struct GrokOAuthPlugin {
    root: usize,
    config: GrokOAuthConfig,
}
impl GrokOAuthPlugin {
    pub fn new(root: usize) -> Self {
        Self::with_config(root, production())
    }
    pub fn with_config(root: usize, config: GrokOAuthConfig) -> Self {
        Self { root, config }
    }
}
pub(crate) fn production() -> GrokOAuthConfig {
    GrokOAuthConfig {
        issuer: Url::parse("https://auth.x.ai").unwrap(),
        client_id: "b1a00492-073a-47ea-816f-4c329264a828".into(),
        scope: "openid profile email offline_access grok-cli:access api:access".into(),
        authorization: None,
        token: Url::parse("https://auth.x.ai/oauth2/token").unwrap(),
        device: Some(Url::parse("https://auth.x.ai/oauth2/device/code").unwrap()),
        redirect: None,
        relay: Url::parse("https://api.x.ai/v1").unwrap(),
    }
}
impl TypedPlugin for GrokOAuthPlugin {
    type Deps = (Keyed<FileStorage>, Keyed<crate::methods::ManagementMethods>);
    fn name(&self) -> &str {
        "GrokOAuth"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(files), Keyed(methods)): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Service::new(
                files,
                self.root,
                "xaiGrokBuild",
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
            let service = Arc::new(GrokOAuthService(service));
            management::provide(ctx, &methods, service.clone())?;
            ctx.provide_as(TypeKey::of::<GrokOAuthService>(), service)?;
            Ok(Effect::Done)
        })
    }
}
