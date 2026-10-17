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

/// The owner Provider resolves `secretRef` references under (`credentials`).
const OWNER: &str = "provider";

impl ProviderSecret {
    /// Parses a resolved value at Provider's boundary: for AWS Bedrock auth a
    /// JSON object is `{accessKeyId, secretAccessKey, sessionToken?}`; any
    /// other value is a token.
    fn resolved(value: String, auth: &StaticAuth) -> Result<Self, HostError> {
        if *auth == StaticAuth::AwsBedrock && value.trim_start().starts_with('{') {
            return serde_json::from_str(&value)
                .map(Self::Aws)
                .map_err(|_| failure("Invalid AWS credential value"));
        }
        Ok(Self::Token(value))
    }
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
        let value = self.owned(cancel, move |cancel| Box::pin(async move {
            tokio::select! {
                _ = cancel.cancelled() => Err(HostError::Cancelled("Secret lookup cancelled".into())),
                result = host.0.resolve(OWNER.into(), reference, cancel.clone()) => {
                    // Host diagnostics can contain values. Do not expose them
                    // through the model failure, trace or management protocol.
                    result.map_err(|_| failure("Provider secret lookup failed"))
                }
            }
        })).await?;
        self.check(cancel)?;
        let value = ProviderSecret::resolved(value, auth)?;
        value.validate(auth)?;
        Ok(Some(value))
    }
}
