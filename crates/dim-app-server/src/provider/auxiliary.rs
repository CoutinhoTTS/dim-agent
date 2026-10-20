//! One model call outside any Session for a consumer that cannot hold
//! credentials. The caller owns the choice of model; no event is written.
use super::{ModelPreferences, ProductResolver, ProviderService};
use dim_agent_capabilities::{host::*, model_provider::invoke_with_recovery};
use dim_agent_types::{ModelId, ModelRequest, SystemPrompt};
use futures_util::StreamExt;
use rutis_bridge::session::{Error, HostDispatch, Reply, Value};
use serde::Deserialize;
use serde_json::json;
use std::{future::Future, pin::Pin, sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(super) struct AuxiliaryModel {
    pub(super) owner: Arc<ProviderService>,
    pub(super) host: Arc<ModelHost>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompleteRequest {
    model: ModelInput,
    request: AuxiliaryRequest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelInput {
    provider_id: String,
    model_id: String,
}

impl HostDispatch for AuxiliaryModel {
    fn methods(&self) -> Option<serde_json::Value> {
        Some(json!({"complete": "async"}))
    }

    fn invoke(&self, method: &str, args: Value) -> Reply {
        if method != "complete" {
            return Err(Error::Value(format!(
                "unknown auxiliaryModel method {method}"
            )));
        }
        let mut args = args
            .list()
            .map_err(|_| failure(invalid("Invalid auxiliaryModel arguments")))?
            .into_iter();
        let request = args
            .next()
            .ok_or_else(|| failure(invalid("Missing auxiliaryModel request")))?;
        let CompleteRequest { model, request } = rutis_bridge::exports::decode(request)
            .map_err(|_| failure(invalid("Invalid auxiliaryModel request")))?;
        let model = ModelId {
            provider_id: model.provider_id,
            model_id: model.model_id,
        };
        let cancelled = match args.next() {
            None | Some(Value::Undefined) => None,
            Some(value) => Some(
                rutis_bridge::exports::callback(value)
                    .map_err(|_| failure(invalid("Invalid auxiliaryModel cancellation")))?,
            ),
        };
        if args.next().is_some() {
            return Err(failure(invalid("Too many auxiliaryModel arguments")));
        }
        let auxiliary = self.clone();
        Ok(Value::future(async move {
            let cancel = CancellationToken::new();
            let call = auxiliary.complete(model, request, cancel.clone());
            tokio::pin!(call);
            let reply = if let Some(cancelled) = cancelled {
                tokio::select! {
                    result = &mut call => result,
                    _ = cancelled.invoke::<(), Error>(vec![]) => {
                        cancel.cancel();
                        call.await
                    }
                }
            } else {
                call.await
            }
            .map_err(failure)?;
            rutis_bridge::exports::encode(&reply)
        }))
    }
}

fn failure(error: AuxiliaryError) -> Error {
    let code = match error.code {
        AuxiliaryErrorCode::InvalidRequest => "invalid_request",
        AuxiliaryErrorCode::ModelUnavailable => "model_unavailable",
        AuxiliaryErrorCode::Failed => "failed",
        AuxiliaryErrorCode::Cancelled => "cancelled",
    };
    rutis_bridge::exports::failure("AuxiliaryError", code, error.message, None)
}

impl CompleteAuxiliary for AuxiliaryModel {
    fn complete(
        &self,
        model: ModelId,
        request: AuxiliaryRequest,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<AuxiliaryReply, AuxiliaryError>> + Send + '_>> {
        Box::pin(async move {
            if request.messages.is_empty() {
                return Err(invalid("messages must not be empty"));
            }
            if request.max_output_tokens == Some(0) {
                return Err(invalid("maxOutputTokens must be positive"));
            }
            // Dropping the call cancels the owned work instead of detaching it.
            let call = cancel.child_token();
            let _dropped = call.clone().drop_guard();
            let (owner, host) = (self.owner.clone(), self.host.clone());
            let target = model.clone();
            let started = Instant::now();
            let result = self
                .owner
                .owned(&call, move |cancel| {
                    Box::pin(async move { Ok(run(owner, host, target, request, cancel).await) })
                })
                .await
                .map_err(Outcome::Host)
                .and_then(|result| result);
            let outcome = match &result {
                Ok(_) => "completed",
                Err(Outcome::Unavailable) => "model_unavailable",
                Err(Outcome::Host(HostError::Cancelled(_))) => "cancelled",
                Err(Outcome::Host(_)) => "failed",
            };
            tracing::info!(
                target: "auxiliary.complete",
                provider_id = %model.provider_id,
                model_id = %model.model_id,
                outcome,
                duration_ms = started.elapsed().as_millis() as u64,
                prompt_tokens = result.as_ref().ok().and_then(|r| r.usage.as_ref()).map(|u| u.prompt_tokens),
                completion_tokens = result.as_ref().ok().and_then(|r| r.usage.as_ref()).map(|u| u.completion_tokens),
            );
            result.map_err(|outcome| match outcome {
                Outcome::Unavailable => AuxiliaryError {
                    code: AuxiliaryErrorCode::ModelUnavailable,
                    message: "Model is unavailable".into(),
                },
                Outcome::Host(HostError::Cancelled(message)) => AuxiliaryError {
                    code: AuxiliaryErrorCode::Cancelled,
                    message,
                },
                Outcome::Host(HostError::Failed {
                    kind: HostErrorKind::OwnerClosed,
                    message,
                    ..
                }) => AuxiliaryError {
                    code: AuxiliaryErrorCode::Cancelled,
                    message,
                },
                Outcome::Host(error) => AuxiliaryError {
                    code: AuxiliaryErrorCode::Failed,
                    message: error.to_string(),
                },
            })
        })
    }
}

enum Outcome {
    Unavailable,
    Host(HostError),
}
impl From<HostError> for Outcome {
    fn from(error: HostError) -> Self {
        Self::Host(error)
    }
}

fn invalid(message: &str) -> AuxiliaryError {
    AuxiliaryError {
        code: AuxiliaryErrorCode::InvalidRequest,
        message: message.into(),
    }
}

async fn run(
    owner: Arc<ProviderService>,
    host: Arc<ModelHost>,
    model: ModelId,
    request: AuxiliaryRequest,
    cancel: CancellationToken,
) -> Result<AuxiliaryReply, Outcome> {
    let selectable = owner
        .catalog_for(model.provider_id.clone(), &cancel)
        .await?
        .is_some_and(|catalog| catalog.selectable(&model.model_id).is_some());
    if !selectable {
        return Err(Outcome::Unavailable);
    }
    let selection = owner
        .configuration(Some(model), None, ModelPreferences::default(), &cancel)
        .await?;
    let limit = selection
        .capability_snapshot
        .as_ref()
        .map(|snapshot| snapshot.capabilities.max_output_tokens)
        .ok_or_else(|| HostError::failed("Model capabilities are missing"))?;
    let mut config = selection.config.unwrap_or_default();
    config.max_tokens = Some(
        request
            .max_output_tokens
            .map_or(limit, |max| u64::from(max).min(limit)) as f64,
    );
    let mut model_request =
        ModelRequest::new(selection.provider_id, selection.model_id, request.messages);
    model_request.system = request
        .system
        .map(|text| Arc::new(SystemPrompt::Text(text)));
    model_request.capability_snapshot = selection.capability_snapshot;
    model_request.config = Some(config);
    let model_request = Arc::new(model_request);
    let resolved = ResolveModelConnection::resolve(
        &ProductResolver(owner.clone()),
        model_request.clone(),
        cancel.clone(),
    )
    .await?;
    // The same failure policy as Session requests, settlement included.
    Ok(invoke_with_recovery(
        &host,
        &model_request,
        resolved,
        &cancel,
        &mut (),
        |stream, _, _, _| Box::pin(collect(stream)),
    )
    .await?)
}

async fn collect(mut stream: ModelStream) -> Result<AuxiliaryReply, HostError> {
    let (mut text, mut usage, mut stop_reason) = (String::new(), None, None);
    while let Some(chunk) = stream.next().await {
        match chunk? {
            ModelChunk::Text(delta) => text.push_str(&delta),
            ModelChunk::Usage(value) => usage = Some(value),
            ModelChunk::MessageStop { reason, .. } => stop_reason = Some(reason),
            _ => {}
        }
    }
    let stop_reason =
        stop_reason.ok_or_else(|| HostError::failed("Model stream ended without a stop"))?;
    // No tools are offered, so a tool stop is a provider defect.
    if stop_reason == ModelStopReason::ToolUse {
        return Err(HostError::failed(
            "Model stopped for a tool it was not offered",
        ));
    }
    Ok(AuxiliaryReply {
        text,
        stop_reason,
        usage,
    })
}
