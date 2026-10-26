//! One MCP server connection. No model requests, permissions or tool registry.
mod sse;
mod stdio;

use super::auth::{Credential, McpAuth, Refusal, credential_key};
use super::config::{BearerAuth, McpServerSettings, RemoteTransport};
use dim_agent_runtime::StateCell;
use dim_agent_types::{
    ContentBlock, ImageSource, ImageSourceType, JsonObject, JsonSchema, ToolContent, ToolResult,
};
use futures_util::StreamExt;
use reqwest_mcp::{
    Client,
    header::{HeaderName, HeaderValue},
};
use rmcp::{
    ClientHandler, Peer, RoleClient, ServiceExt,
    model::{
        CallToolRequest, CallToolRequestParams, ClientJsonRpcMessage, ClientRequest,
        ListToolsRequest, PaginatedRequestParams, ServerResult,
    },
    service::{
        NotificationContext, PeerRequestOptions, RunningService, RxJsonRpcMessage, ServiceError,
        TxJsonRpcMessage,
    },
    transport::{
        StreamableHttpClientTransport, Transport,
        common::client_side_sse::BoxedSseResponse,
        streamable_http_client::{
            AuthRequiredError, StreamableHttpClient, StreamableHttpClientTransportConfig,
            StreamableHttpError, StreamableHttpPostResponse,
        },
    },
};
use rutis::host::{CtxHost, Key};
use rutis::{Ctx, Plugin};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Debug, thiserror::Error)]
pub enum McpServerError {
    #[error("MCP server is closed")]
    Closed,
    #[error("MCP operation cancelled; remote effects may already have occurred")]
    Cancelled,
    #[error("MCP operation timed out; remote effects may already have occurred")]
    Timeout,
    #[error("invalid MCP configuration: {0}")]
    Configuration(&'static str),
    #[error("MCP connection or request failed; the operation was not replayed")]
    RequestFailed,
    #[error("invalid MCP server response")]
    InvalidResponse,
    #[error("MCP cleanup failed")]
    Cleanup,
    #[error("MCP server authorization required")]
    AuthRequired { expired: bool },
    /// A `403 insufficient_scope`, with the challenged scopes.
    #[error("MCP server requires additional scopes")]
    InsufficientScope(String),
}
pub type Result<T> = std::result::Result<T, McpServerError>;

#[derive(Clone, Debug, PartialEq)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: JsonSchema,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpServerState {
    Disconnected,
    Connecting,
    Connected,
    Failed,
    Closed,
    /// The server refused the request for lack of a usable OAuth credential.
    /// The next request reconnects, for example once the user signs in.
    AuthRequired {
        expired: bool,
    },
}

struct Notifications(StateCell<u64>);
impl ClientHandler for Notifications {
    async fn on_tool_list_changed(&self, _: NotificationContext<RoleClient>) {
        self.0.update(|revision| *revision += 1);
    }
}

type HttpError = StreamableHttpError<reqwest_mcp::Error>;

/// A connection's OAuth credential, and the state its refusals set.
#[derive(Clone)]
pub(super) struct Authorization {
    credential: Credential,
    server: reqwest_mcp::Url,
    state: StateCell<McpServerState>,
}
impl Authorization {
    fn refused(&self, refusal: Refusal) -> McpServerError {
        match refusal {
            Refusal::Unavailable => McpServerError::RequestFailed,
            Refusal::Required | Refusal::Expired => {
                let expired = matches!(refusal, Refusal::Expired);
                self.state.set(McpServerState::AuthRequired { expired });
                McpServerError::AuthRequired { expired }
            }
        }
    }
    /// Only `error="insufficient_scope"` is step-up; rmcp reports any `403`
    /// with a challenge as insufficient scope. Refused during the handshake,
    /// the connection cannot be used until the user signs in again.
    fn insufficient_scope(&self, header: &str) -> Option<String> {
        let scopes = self.credential.insufficient_scope(header, &self.server)?;
        self.state.update(|state| {
            if *state == McpServerState::Connecting {
                *state = McpServerState::AuthRequired { expired: false };
            }
        });
        Some(scopes.join(" "))
    }
}

// rmcp's bootstrap POSTs are inline awaits which do not observe the worker
// cancellation token. Cancel their HTTP futures (including an initial SSE
// response) explicitly. After initialization this token remains uncancelled;
// normal close uses rmcp's graceful request draining and session DELETE.
// An OAuth connection attaches a bearer to every request (design §5.6).
#[derive(Clone)]
struct InitializingClient {
    client: Client,
    cancel: CancellationToken,
    authorization: Option<Authorization>,
}
impl InitializingClient {
    async fn authorized<T, Fut>(
        &self,
        auth_header: Option<String>,
        call: impl Fn(Option<String>) -> Fut,
    ) -> std::result::Result<T, HttpError>
    where
        Fut: Future<Output = std::result::Result<T, HttpError>>,
    {
        let Some(authorization) = &self.authorization else {
            return match call(auth_header).await {
                Err(HttpError::InsufficientScope(_)) => Err(forbidden()),
                result => result,
            };
        };
        let result = authorization
            .credential
            .send(call, |result| match result {
                Err(HttpError::AuthRequired(error)) => {
                    Some(Some(error.www_authenticate_header.clone()))
                }
                Err(HttpError::Client(error))
                    if error.status() == Some(reqwest_mcp::StatusCode::UNAUTHORIZED) =>
                {
                    Some(None)
                }
                // rmcp reports a POST `401` without `WWW-Authenticate` as text.
                Err(HttpError::UnexpectedServerResponse(message))
                    if message.starts_with("HTTP 401 ") =>
                {
                    Some(None)
                }
                _ => None,
            })
            .await
            .map_err(|refusal| match authorization.refused(refusal) {
                McpServerError::AuthRequired { .. } => {
                    HttpError::AuthRequired(AuthRequiredError::new(String::new()))
                }
                _ => HttpError::UnexpectedServerResponse("authorization unavailable".into()),
            })?;
        match result {
            Err(HttpError::InsufficientScope(error))
                if authorization
                    .insufficient_scope(&error.www_authenticate_header)
                    .is_none() =>
            {
                Err(forbidden())
            }
            result => result,
        }
    }
}
fn forbidden() -> HttpError {
    HttpError::UnexpectedServerResponse("forbidden".into())
}
impl StreamableHttpClient for InitializingClient {
    type Error = reqwest_mcp::Error;
    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> std::result::Result<StreamableHttpPostResponse, HttpError> {
        self.post_message_with_max_sse_event_size(
            uri,
            message,
            session_id,
            auth_header,
            headers,
            StreamableHttpClientTransportConfig::default().max_sse_event_size,
        )
        .await
    }
    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
        limit: usize,
    ) -> std::result::Result<StreamableHttpPostResponse, HttpError> {
        let post = self.authorized(auth_header, |token| {
            self.client.post_message_with_max_sse_event_size(
                uri.clone(),
                message.clone(),
                session_id.clone(),
                token,
                headers.clone(),
                limit,
            )
        });
        let response = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Err(HttpError::TransportChannelClosed),
            result = post => result?,
        };
        Ok(match response {
            StreamableHttpPostResponse::Sse(stream, session) => StreamableHttpPostResponse::Sse(
                stream
                    .take_until(self.cancel.clone().cancelled_owned())
                    .boxed(),
                session,
            ),
            other => other,
        })
    }
    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> std::result::Result<(), HttpError> {
        self.authorized(auth_header, |token| {
            self.client
                .delete_session(uri.clone(), session_id.clone(), token, headers.clone())
        })
        .await
    }
    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> std::result::Result<BoxedSseResponse, HttpError> {
        self.get_stream_with_max_sse_event_size(
            uri,
            session_id,
            last_event_id,
            auth_header,
            headers,
            StreamableHttpClientTransportConfig::default().max_sse_event_size,
        )
        .await
    }
    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
        limit: usize,
    ) -> std::result::Result<BoxedSseResponse, HttpError> {
        self.authorized(auth_header, |token| {
            self.client.get_stream_with_max_sse_event_size(
                uri.clone(),
                session_id.clone(),
                last_event_id.clone(),
                token,
                headers.clone(),
                limit,
            )
        })
        .await
    }
}

// rmcp consumes its transport during initialization. On handshake failure it
// drops it without awaiting close. Retain an explicit completion signal so
// the owning Plugin can also drain that path, including session DELETE.
struct OwnedTransport<T: Transport<RoleClient> + 'static> {
    inner: Option<T>,
    closed: Option<oneshot::Sender<Result<()>>>,
    state: StateCell<McpServerState>,
}
impl<T: Transport<RoleClient> + 'static> Transport<RoleClient> for OwnedTransport<T> {
    type Error = T::Error;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = std::result::Result<(), T::Error>> + Send + 'static {
        self.inner.as_mut().expect("live transport").send(item)
    }
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        let message = self.inner.as_mut().expect("live transport").receive().await;
        if message.is_none() {
            self.state.update(|state| {
                if !matches!(state, McpServerState::AuthRequired { .. }) {
                    *state = McpServerState::Failed;
                }
            });
        }
        message
    }
    async fn close(&mut self) -> std::result::Result<(), T::Error> {
        let result = if let Some(inner) = self.inner.as_mut() {
            inner.close().await
        } else {
            Ok(())
        };
        self.inner = None;
        if let Some(closed) = self.closed.take() {
            let _ = closed.send(
                result
                    .as_ref()
                    .map(|_| ())
                    .map_err(|_| McpServerError::Cleanup),
            );
        }
        result
    }
}
impl<T: Transport<RoleClient> + 'static> Drop for OwnedTransport<T> {
    fn drop(&mut self) {
        if let Some(mut inner) = self.inner.take() {
            let closed = self.closed.take();
            tokio::spawn(async move {
                let result = inner.close().await.map_err(|_| McpServerError::Cleanup);
                if let Some(closed) = closed {
                    let _ = closed.send(result);
                }
            });
        }
    }
}

struct Connection {
    running: RunningService<RoleClient, Notifications>,
    closed: oneshot::Receiver<Result<()>>,
}
impl Connection {
    async fn close(mut self) -> Result<()> {
        let result = self.running.close().await;
        self.closed.await.map_err(|_| McpServerError::Cleanup)??;
        result.map_err(|_| McpServerError::Cleanup)?;
        Ok(())
    }
}

#[derive(Clone)]
enum TransportConfig {
    Http(StreamableHttpClientTransportConfig),
    Sse(StreamableHttpClientTransportConfig),
    Stdio(stdio::Config),
}

pub struct McpServer {
    transport: TransportConfig,
    authorization: Option<Authorization>,
    state: StateCell<McpServerState>,
    tool_changes: StateCell<u64>,
    connection: AsyncMutex<Option<Connection>>,
    stopping: CancellationToken,
    gate: Mutex<()>,
    tasks: TaskTracker,
}
impl McpServer {
    pub fn state(&self) -> McpServerState {
        self.state.get()
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<McpServerState> {
        self.state.subscribe()
    }
    pub fn tool_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.tool_changes.subscribe()
    }
    /// Changes when this server's OAuth credential changes; `None` without
    /// OAuth.
    pub fn credential_changes(&self) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.authorization
            .as_ref()
            .map(|authorization| authorization.credential.subscribe())
    }

    /// Fetch every page; reject cycles/duplicate names instead of publishing a partial set.
    pub async fn tools(self: &Arc<Self>, cancel: &CancellationToken) -> Result<Vec<McpTool>> {
        let owner = self.clone();
        self.run(cancel, move |cancel| async move {
            let mut cursor = None;
            let mut cursors = BTreeSet::new();
            let mut names = BTreeSet::new();
            let mut tools = Vec::new();
            loop {
                let params = PaginatedRequestParams::default().with_cursor(cursor);
                let request = ClientRequest::ListToolsRequest(ListToolsRequest::with_param(params));
                let ServerResult::ListToolsResult(page) = owner.request(request, &cancel).await?
                else {
                    return Err(McpServerError::InvalidResponse);
                };
                for tool in page.tools {
                    if tool.name.is_empty()
                        || !names.insert(tool.name.to_string())
                        || tool.input_schema.get("type").and_then(|v| v.as_str()) != Some("object")
                    {
                        return Err(McpServerError::InvalidResponse);
                    }
                    tools.push(McpTool {
                        name: tool.name.into_owned(),
                        description: tool.description.map(|s| s.into_owned()),
                        input_schema: serde_json::from_value(serde_json::Value::Object(
                            (*tool.input_schema).clone(),
                        ))
                        .map_err(|_| McpServerError::InvalidResponse)?,
                    });
                }
                cursor = page.next_cursor;
                let Some(next) = cursor.as_ref() else {
                    return Ok(tools);
                };
                if !cursors.insert(next.clone()) {
                    return Err(McpServerError::InvalidResponse);
                }
            }
        })
        .await
    }

    /// Server-local name. Session namespacing and request binding belong to the consumer.
    pub async fn call(
        self: &Arc<Self>,
        name: String,
        arguments: JsonObject,
        cancel: &CancellationToken,
    ) -> Result<ToolResult> {
        let owner = self.clone();
        self.run(cancel, move |cancel| async move {
            let mut params = CallToolRequestParams::new(name);
            params.arguments = Some(arguments);
            let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
            let ServerResult::CallToolResult(result) = owner.request(request, &cancel).await?
            else {
                return Err(McpServerError::InvalidResponse);
            };
            let mut content = Vec::new();
            for block in result.content {
                content.push(match block {
                    rmcp::model::ContentBlock::Text(text) => ContentBlock::Text { text: text.text },
                    rmcp::model::ContentBlock::Image(image) => ContentBlock::Image {
                        source: ImageSource {
                            source_type: ImageSourceType::Base64,
                            media_type: image.mime_type,
                            data: Some(image.data),
                            url: None,
                        },
                    },
                    other => ContentBlock::Text {
                        text: serde_json::to_string(&other)
                            .map_err(|_| McpServerError::InvalidResponse)?,
                    },
                });
            }
            // structuredContent is not model-readable. Preserve its handles in
            // content as well, including structured-only MCP responses.
            if let Some(value) = &result.structured_content {
                content.push(ContentBlock::Text {
                    text: value.to_string(),
                });
            }
            Ok(ToolResult {
                content: ToolContent::Blocks(content),
                structured_content: result
                    .structured_content
                    .and_then(|v| v.as_object().cloned()),
                is_error: result.is_error,
                ..ToolResult::default()
            })
        })
        .await
    }

    async fn peer(&self, cancel: &CancellationToken) -> Result<Peer<RoleClient>> {
        let mut connection = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(McpServerError::Cancelled),
            guard = self.connection.lock() => guard,
        };
        // Refused for lack of a credential: start over with a new connection.
        if matches!(self.state.get(), McpServerState::AuthRequired { .. })
            && let Some(refused) = connection.take()
        {
            let _ = refused.close().await;
        }
        if let Some(connection) = connection.as_ref() {
            if connection.running.is_closed() {
                self.state.set(McpServerState::Failed);
                return Err(McpServerError::RequestFailed);
            }
            return Ok(connection.running.peer().clone());
        }
        // Failed connections require an explicit directory disconnect/reopen.
        if self.state.get() == McpServerState::Failed {
            return Err(McpServerError::RequestFailed);
        }
        self.state.set(McpServerState::Connecting);
        let result = self.connect(cancel).await;
        match result {
            Ok(connected) => {
                let peer = connected.running.peer().clone();
                *connection = Some(connected);
                self.state.update(|state| {
                    if *state == McpServerState::Connecting {
                        *state = McpServerState::Connected;
                    }
                });
                Ok(peer)
            }
            Err(error) => {
                if let McpServerState::AuthRequired { expired } = self.state.get() {
                    return Err(McpServerError::AuthRequired { expired });
                }
                self.state.set(McpServerState::Failed);
                Err(error)
            }
        }
    }

    async fn connect(&self, cancel: &CancellationToken) -> Result<Connection> {
        let handshake_cancel = CancellationToken::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        match &self.transport {
            TransportConfig::Http(config) => {
                let transport = StreamableHttpClientTransport::with_client(
                    InitializingClient {
                        client: http_client()?,
                        cancel: handshake_cancel.clone(),
                        authorization: self.authorization.clone(),
                    },
                    config.clone(),
                );
                self.initialize(transport, cancel, handshake_cancel, deadline)
                    .await
            }
            TransportConfig::Sse(config) => {
                let transport = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(McpServerError::Cancelled),
                    _ = tokio::time::sleep_until(deadline) => return Err(McpServerError::Timeout),
                    result = sse::SseTransport::connect(http_client()?, config, self.authorization.clone(), handshake_cancel.clone()) => result?,
                };
                self.initialize(transport, cancel, handshake_cancel, deadline)
                    .await
            }
            TransportConfig::Stdio(config) => {
                self.initialize(config.spawn()?, cancel, handshake_cancel, deadline)
                    .await
            }
        }
    }

    async fn initialize<T: Transport<RoleClient> + 'static>(
        &self,
        transport: T,
        cancel: &CancellationToken,
        handshake_cancel: CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<Connection> {
        let (tx, closed) = oneshot::channel();
        let transport = OwnedTransport {
            inner: Some(transport),
            closed: Some(tx),
            state: self.state.clone(),
        };
        let lifetime = CancellationToken::new();
        let initialize =
            Notifications(self.tool_changes.clone()).serve_with_ct(transport, lifetime.clone());
        tokio::pin!(initialize);
        // Poll the handshake before honoring cancellation: rmcp's HTTP worker
        // waits for its first message without observing its cancellation
        // token, so a transport closed before sending one never finishes
        // closing.
        let failure = tokio::select! {
            biased;
            result = &mut initialize => {
                match result {
                    Ok(running) => {
                        return Ok(Connection { running, closed });
                    },
                    Err(_) => {
                        handshake_cancel.cancel();
                        closed.await.map_err(|_| McpServerError::Cleanup)??;
                        return Err(McpServerError::RequestFailed);
                    }
                }
            }
            _ = cancel.cancelled() => McpServerError::Cancelled,
            _ = tokio::time::sleep_until(deadline) => McpServerError::Timeout,
        };
        handshake_cancel.cancel();
        lifetime.cancel();
        if let Ok(running) = initialize.await {
            Connection { running, closed }.close().await?;
        } else {
            closed.await.map_err(|_| McpServerError::Cleanup)??;
        }
        Err(failure)
    }

    async fn request(
        &self,
        request: ClientRequest,
        cancel: &CancellationToken,
    ) -> Result<ServerResult> {
        let peer = self.peer(cancel).await?;
        if cancel.is_cancelled() {
            return Err(McpServerError::Cancelled);
        }
        // Do not drop request admission: once accepted, retain its request id
        // so cancellation can be sent to the peer, even if the caller leaves.
        let mut handle = peer
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await
            .map_err(|_| McpServerError::RequestFailed)?;
        let failure = tokio::select! {
            biased;
            _ = cancel.cancelled() => McpServerError::Cancelled,
            _ = tokio::time::sleep(Duration::from_secs(60)) => McpServerError::Timeout,
            response = &mut handle.rx => return response.map_err(|_| McpServerError::RequestFailed)?.map_err(|error| self.failure(&error)),
        };
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            handle.cancel(Some("MCP operation cancelled".into())),
        )
        .await;
        Err(failure)
    }

    /// Authorization outcomes of a request, from the transport's own error.
    fn failure(&self, error: &ServiceError) -> McpServerError {
        let ServiceError::TransportSend(error) = error else {
            return McpServerError::RequestFailed;
        };
        let refused = match self.state.get() {
            McpServerState::AuthRequired { expired } => McpServerError::AuthRequired { expired },
            _ => McpServerError::RequestFailed,
        };
        match error.error.downcast_ref::<HttpError>() {
            Some(HttpError::AuthRequired(_)) => return refused,
            Some(HttpError::InsufficientScope(error)) => {
                return McpServerError::InsufficientScope(
                    error.required_scope.clone().unwrap_or_default(),
                );
            }
            _ => {}
        }
        match error.error.downcast_ref::<McpServerError>() {
            Some(McpServerError::AuthRequired { .. }) => refused,
            Some(McpServerError::InsufficientScope(scopes)) => {
                McpServerError::InsufficientScope(scopes.clone())
            }
            _ => McpServerError::RequestFailed,
        }
    }

    async fn run<T, F, Fut>(&self, cancel: &CancellationToken, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        let task = {
            let _gate = self.gate.lock().unwrap();
            if self.stopping.is_cancelled() {
                return Err(McpServerError::Closed);
            }
            if cancel.is_cancelled() {
                return Err(McpServerError::Cancelled);
            }
            let stopping = self.stopping.clone();
            let cancel = cancel.child_token();
            self.tasks.spawn(async move {
                let operation = operation(cancel.clone());
                tokio::pin!(operation);
                tokio::select! {
                    biased;
                    _ = stopping.cancelled() => {},
                    result = &mut operation => return result,
                }
                cancel.cancel();
                operation.await
            })
        };
        task.await.map_err(|_| McpServerError::RequestFailed)?
    }
}

fn http_configuration(
    url: &str,
    headers: &std::collections::BTreeMap<String, String>,
    auth: &Option<BearerAuth>,
) -> Result<StreamableHttpClientTransportConfig> {
    let uri =
        reqwest_mcp::Url::parse(url).map_err(|_| McpServerError::Configuration("invalid URL"))?;
    if !matches!(uri.scheme(), "http" | "https")
        || !uri.username().is_empty()
        || uri.password().is_some()
        || uri.fragment().is_some()
    {
        return Err(McpServerError::Configuration(
            "expected HTTP URL without userinfo or fragment",
        ));
    }
    let mut custom_headers = HashMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| McpServerError::Configuration("invalid header name"))?;
        if matches!(
            name.as_str(),
            "accept" | "mcp-session-id" | "mcp-protocol-version" | "last-event-id"
        ) {
            return Err(McpServerError::Configuration("reserved header name"));
        }
        let mut value = HeaderValue::from_str(value)
            .map_err(|_| McpServerError::Configuration("invalid header value"))?;
        value.set_sensitive(true);
        if custom_headers.insert(name, value).is_some() {
            return Err(McpServerError::Configuration("duplicate header name"));
        }
    }
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_owned())
        .custom_headers(custom_headers)
        .reinit_on_expired_session(false);
    if let Some(BearerAuth::Bearer { token }) = auth {
        if headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
        {
            return Err(McpServerError::Configuration("ambiguous authorization"));
        }
        HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| McpServerError::Configuration("invalid bearer token"))?;
        config = config.auth_header(token.clone());
    }
    Ok(config)
}

fn http_client() -> Result<Client> {
    Client::builder()
        .redirect(reqwest_mcp::redirect::Policy::none())
        .retry(reqwest_mcp::retry::never())
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| McpServerError::Configuration("HTTP client initialization failed"))
}

pub(super) struct McpServerPlugin {
    pub key: Key<McpServer>,
    transport: TransportConfig,
    oauth: Option<(super::auth::CredentialKey, reqwest_mcp::Url)>,
}
impl McpServerPlugin {
    pub fn new(id: &str, configuration: McpServerSettings, key: Key<McpServer>) -> Result<Self> {
        let oauth = credential_key(id, &configuration);
        let transport = match &configuration {
            McpServerSettings::Stdio {
                command,
                args,
                env,
                cwd,
                ..
            } => TransportConfig::Stdio(stdio::Config::new(command, args, env, cwd)?),
            McpServerSettings::Remote {
                kind,
                url,
                headers,
                auth,
                ..
            } => {
                let config = http_configuration(url, headers, auth)?;
                match kind {
                    RemoteTransport::Http => TransportConfig::Http(config),
                    RemoteTransport::Sse => TransportConfig::Sse(config),
                }
            }
        };
        Ok(Self {
            key,
            transport,
            oauth,
        })
    }
}
impl Plugin for McpServerPlugin {
    // Also orders this server inside the directory's lifetime. McpAuth is a
    // process name, whose key is its type's.
    type Deps = Arc<McpAuth>;
    fn name(&self) -> &str {
        "McpServer"
    }
    async fn apply(&self, ctx: &Ctx, auth: Arc<McpAuth>) -> rutis::Result {
        let state = StateCell::new(McpServerState::Disconnected);
        let service = Arc::new(McpServer {
            transport: self.transport.clone(),
            authorization: self.oauth.clone().map(|(key, server)| Authorization {
                credential: auth.credential(key),
                server,
                state: state.clone(),
            }),
            state,
            tool_changes: StateCell::new(0),
            connection: AsyncMutex::new(None),
            stopping: ctx.cancellation_token(),
            gate: Mutex::new(()),
            tasks: TaskTracker::new(),
        });
        let owner = service.clone();
        ctx.effect(move || async move {
            {
                let _gate = owner.gate.lock().unwrap();
                owner.tasks.close();
            }
            owner.tasks.wait().await;
            let connection = owner.connection.lock().await.take();
            let result = match connection {
                Some(connection) => connection.close().await,
                None => Ok(()),
            };
            owner.state.set(if result.is_ok() {
                McpServerState::Closed
            } else {
                McpServerState::Failed
            });
            Ok(result?)
        })?;
        ctx.provide_at(self.key.clone(), service)?;
        Ok(())
    }
}
