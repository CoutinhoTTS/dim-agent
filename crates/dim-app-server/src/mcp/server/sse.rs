//! Legacy HTTP+SSE: one GET event stream and a server-advertised POST endpoint.
//! rmcp still owns JSON-RPC, negotiation, correlation and cancellation messages.
use super::{Authorization, HttpError, McpServerError, Result};
use futures_util::StreamExt;
use reqwest_mcp::{
    Client, StatusCode, Url,
    header::{HeaderMap, WWW_AUTHENTICATE},
};
use rmcp::{
    RoleClient,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{
        Transport,
        common::client_side_sse::BoxedSseResponse,
        streamable_http_client::{StreamableHttpClient, StreamableHttpClientTransportConfig},
    },
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub(super) struct SseTransport {
    client: Client,
    endpoint: Url,
    headers: HeaderMap,
    auth: Option<String>,
    authorization: Option<Authorization>,
    stream: Option<BoxedSseResponse>,
    stopping: CancellationToken,
}
impl SseTransport {
    // No worker is spawned here: dropping this future cancels GET/discovery.
    pub async fn connect(
        client: Client,
        config: &StreamableHttpClientTransportConfig,
        authorization: Option<Authorization>,
        stopping: CancellationToken,
    ) -> Result<Self> {
        let base =
            Url::parse(&config.uri).map_err(|_| McpServerError::Configuration("invalid URL"))?;
        // Reuse the SDK's bounded SSE decoder directly, without its Streamable
        // HTTP worker or reconnect policy. It preserves endpoint events.
        let get = |token| {
            client.get_stream_with_max_sse_event_size(
                config.uri.clone(),
                None,
                None,
                token,
                config.custom_headers.clone(),
                config.max_sse_event_size,
            )
        };
        let stream = match &authorization {
            None => get(config.auth_header.clone()).await,
            Some(authorization) => authorization
                .credential
                .send(get, |result| match result {
                    Err(HttpError::AuthRequired(error)) => {
                        Some(Some(error.www_authenticate_header.clone()))
                    }
                    Err(HttpError::Client(error))
                        if error.status() == Some(StatusCode::UNAUTHORIZED) =>
                    {
                        Some(None)
                    }
                    _ => None,
                })
                .await
                .map_err(|refusal| authorization.refused(refusal))?,
        };
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(HttpError::InsufficientScope(error)) => {
                return Err(authorization
                    .as_ref()
                    .and_then(|a| a.insufficient_scope(&error.www_authenticate_header))
                    .map_or(
                        McpServerError::RequestFailed,
                        McpServerError::InsufficientScope,
                    ));
            }
            Err(_) => return Err(McpServerError::RequestFailed),
        };
        let endpoint = loop {
            let event = stream
                .next()
                .await
                .ok_or(McpServerError::RequestFailed)?
                .map_err(|_| McpServerError::InvalidResponse)?;
            match event.event.as_deref() {
                Some("endpoint") => {
                    let value = event.data.ok_or(McpServerError::InvalidResponse)?;
                    let endpoint = base
                        .join(&value)
                        .map_err(|_| McpServerError::InvalidResponse)?;
                    // Never forward configured secrets to a different origin,
                    // downgrade HTTPS, or accept URL-embedded credentials.
                    if value.trim().is_empty()
                        || endpoint.origin() != base.origin()
                        || !endpoint.username().is_empty()
                        || endpoint.password().is_some()
                        || endpoint.fragment().is_some()
                    {
                        return Err(McpServerError::InvalidResponse);
                    }
                    break endpoint;
                }
                Some("message") => return Err(McpServerError::InvalidResponse),
                _ => {}
            }
        };
        Ok(Self {
            client,
            endpoint,
            headers: config.custom_headers.clone().into_iter().collect(),
            auth: config.auth_header.clone(),
            authorization,
            stream: Some(stream),
            stopping,
        })
    }
}
impl Transport<RoleClient> for SseTransport {
    type Error = McpServerError;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<()>> + Send + 'static {
        let (client, endpoint, headers) = (
            self.client.clone(),
            self.endpoint.clone(),
            self.headers.clone(),
        );
        let (auth, authorization) = (self.auth.clone(), self.authorization.clone());
        let stopping = self.stopping.clone();
        async move {
            let post = |token: Option<String>| {
                let request = client
                    .post(endpoint.clone())
                    .headers(headers.clone())
                    .timeout(Duration::from_secs(60))
                    .json(&item);
                match token {
                    Some(token) => request.bearer_auth(token).send(),
                    None => request.send(),
                }
            };
            let send = async {
                match &authorization {
                    None => Ok(post(auth).await),
                    Some(authorization) => authorization
                        .credential
                        .send(post, |result| match result {
                            Ok(response) if response.status() == StatusCode::UNAUTHORIZED => {
                                Some(challenge(response))
                            }
                            _ => None,
                        })
                        .await
                        .map_err(|refusal| authorization.refused(refusal)),
                }
            };
            let response = tokio::select! {
                biased;
                _ = stopping.cancelled() => return Err(McpServerError::Closed),
                response = send => response?.map_err(|_| McpServerError::RequestFailed)?,
            };
            if response.status() == StatusCode::FORBIDDEN
                && let Some(scopes) = challenge(&response).and_then(|header| {
                    authorization
                        .as_ref()
                        .and_then(|authorization| authorization.insufficient_scope(&header))
                })
            {
                return Err(McpServerError::InsufficientScope(scopes));
            }
            if !response.status().is_success() {
                return Err(McpServerError::RequestFailed);
            }
            // JSON-RPC replies arrive on GET, not in this acknowledgement body.
            Ok(())
        }
    }
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        loop {
            let next = tokio::select! {
                biased;
                _ = self.stopping.cancelled() => return None,
                next = self.stream.as_mut()?.next() => next,
            };
            match next {
                Some(Ok(event)) => match event.event.as_deref() {
                    Some("message" | "") | None => {
                        // SSE defaults to message when the event field is absent
                        // or empty. An event with no data is not dispatched.
                        let Some(data) = event.data else { continue };
                        if let Ok(message) = serde_json::from_str(&data) {
                            return Some(message);
                        }
                        break;
                    }
                    Some("endpoint") => break, // A live route is immutable.
                    _ => {}
                },
                _ => break,
            }
        }
        self.stopping.cancel();
        None
    }
    async fn close(&mut self) -> Result<()> {
        self.stopping.cancel();
        self.stream.take();
        Ok(())
    }
}

fn challenge(response: &reqwest_mcp::Response) -> Option<String> {
    response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}
