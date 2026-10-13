//! External secret boundary. Storage retains references, never resolved values.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProviderSecret {
    Token(String),
    Aws(AwsCredentials),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(super) enum StaticCredential {
    Inline(ProviderSecret),
    Reference {
        #[serde(rename = "secretRef")]
        reference: String,
    },
}

/// Implemented by the plugin that owns the host secret store. The future must
/// be cancellation-safe; it must not detach work or return secrets in errors.
pub trait ResolveProviderSecret: Send + Sync + 'static {
    fn resolve(
        &self,
        provider_id: String,
        reference: String,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ProviderSecret>;
}

pub struct ProviderSecretHost(pub Arc<dyn ResolveProviderSecret>);

impl ProviderSecret {
    pub(super) fn validate(&self, auth: &StaticAuth) -> Result<(), HostError> {
        match (auth, self) {
            (StaticAuth::AwsBedrock, Self::Aws(credentials)) => {
                for value in [
                    Some(credentials.access_key_id.as_str()),
                    Some(credentials.secret_access_key.as_str()),
                    credentials.session_token.as_deref(),
                ]
                .into_iter()
                .flatten()
                {
                    if value.is_empty() || reqwest::header::HeaderValue::from_str(value).is_err() {
                        return Err(failure("Invalid AWS credential value"));
                    }
                }
            }
            (_, Self::Token(value)) => {
                if value.is_empty() {
                    return Err(failure("Credential must not be empty"));
                }
                let prefix = match auth {
                    StaticAuth::ApiKey { prefix, .. } => prefix.as_str(),
                    StaticAuth::Bearer | StaticAuth::AwsBedrock => "Bearer ",
                    _ => "",
                };
                reqwest::header::HeaderValue::from_str(&format!("{prefix}{value}"))
                    .map_err(|_| failure("Invalid credential header value"))?;
            }
            _ => {
                return Err(failure(
                    "Structured AWS credentials require AWS Bedrock auth",
                ));
            }
        }
        Ok(())
    }
}

impl ProviderService {
    pub(super) async fn credential(
        self: &Arc<Self>,
        provider_id: &str,
        credential: &Option<StaticCredential>,
        auth: &StaticAuth,
        cancel: &CancellationToken,
    ) -> Result<Option<ProviderSecret>, HostError> {
        if *auth == StaticAuth::Anonymous {
            return Ok(None);
        }
        let reference = match credential {
            None => return Ok(None),
            Some(StaticCredential::Inline(value)) => return Ok(Some(value.clone())),
            Some(StaticCredential::Reference { reference }) => reference.clone(),
        };
        let host = self.secret_host.clone().ok_or_else(|| {
            HostError::classified(
                HostErrorKind::Unsupported,
                "No provider secret store configured",
            )
        })?;
        let provider_id = provider_id.to_owned();
        let value = self.owned(cancel, move |cancel| Box::pin(async move {
            tokio::select! {
                _ = cancel.cancelled() => Err(HostError::Cancelled("Secret lookup cancelled".into())),
                result = host.0.resolve(provider_id, reference, cancel.clone()) => {
                    // Host diagnostics can contain values. Do not expose them
                    // through the model failure, trace or management protocol.
                    result.map_err(|_| failure("Provider secret lookup failed"))
                }
            }
        })).await?;
        self.check(cancel)?;
        value.validate(auth)?;
        Ok(Some(value))
    }
}
