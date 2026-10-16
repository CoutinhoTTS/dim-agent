//! The `sessions` process service, see
//! `docs/design/appserver/sessions-service.md`: named, data-only access to
//! Sessions for process plugins in any language. The typed methods live on
//! [`AgentIndex`]; Agents' directory part, which never reloads, serves them by
//! name and looks up the index on each call.
use super::AgentIndex;
use crate::protocol::{PROTOCOL_VERSION, session_cwd};
use dim_agent_capabilities::permission::PermissionState;
use dim_agent_runtime::AgentError;
use dim_agent_types::{ModelId, ModelSelection, PermissionPolicy, PermissionPreset};
use rutis::Ctx;
use rutis_bridge::session::{HostDispatch, Reply, Value};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use std::{future::Future, sync::Arc};

/// A branch of a Session.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionRef {
    pub session_id: String,
    pub branch_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchRequest {
    /// Agent id; absent: the default Agent.
    #[serde(default)]
    pub agent: Option<String>,
    pub cwd: String,
    /// Replaces the Agent profile's model.
    #[serde(default)]
    pub model: Option<ModelId>,
    /// Capped by the bound; absent: the starter's policy, else read-only.
    #[serde(default)]
    pub policy: Option<PermissionPolicy>,
    /// The branch whose current policy bounds the launch; absent: the host's
    /// Session defaults.
    #[serde(default)]
    pub starter: Option<SessionRef>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchReply {
    pub session_id: String,
    /// The effective policy the Session was created with.
    pub policy: PermissionPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionsErrorCode {
    Rejected,
    AgentUnknown,
    SessionClosed,
    BranchMissing,
    Reloading,
    Failed,
}
/// A `sessions` failure, carried as data (session-access A.6).
#[derive(Debug, Clone)]
pub struct SessionsError {
    pub code: SessionsErrorCode,
    pub message: String,
}
impl SessionsError {
    fn new(code: SessionsErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    fn rejected(message: impl Into<String>) -> Self {
        Self::new(SessionsErrorCode::Rejected, message)
    }
    fn reloading(what: &str) -> Self {
        Self::new(SessionsErrorCode::Reloading, format!("{what} is reloading"))
    }
    /// The caller may retry, after `lookup` or `history` where a write may
    /// have committed.
    pub fn retryable(&self) -> bool {
        matches!(self.code, SessionsErrorCode::Reloading)
    }
}
impl From<AgentError> for SessionsError {
    fn from(error: AgentError) -> Self {
        let code = match error {
            AgentError::Closed(_) => SessionsErrorCode::SessionClosed,
            AgentError::UnknownBranch(_) => SessionsErrorCode::BranchMissing,
            AgentError::Reloading(_) => SessionsErrorCode::Reloading,
            AgentError::InvalidInput(_) => SessionsErrorCode::Rejected,
            _ => SessionsErrorCode::Failed,
        };
        Self::new(code, error.to_string())
    }
}

/// The launch cap (sessions service §6.1): the lower preset, the union of the
/// disallowed tools, and only the allowed tools both sides allow
/// unconditionally. It is never wider than either side.
fn cap(requested: &PermissionPolicy, bound: &PermissionPolicy) -> PermissionPolicy {
    fn rank(preset: PermissionPreset) -> u8 {
        match preset {
            PermissionPreset::ReadOnly => 0,
            PermissionPreset::WorkspaceWrite => 1,
            PermissionPreset::FullAccess => 2,
        }
    }
    fn allows(policy: &PermissionPolicy, tool: &String) -> bool {
        policy.preset == PermissionPreset::FullAccess || policy.allowed_tools.contains(tool)
    }
    let mut disallowed_tools = Vec::new();
    for tool in requested
        .disallowed_tools
        .iter()
        .chain(&bound.disallowed_tools)
    {
        if !disallowed_tools.contains(tool) {
            disallowed_tools.push(tool.clone());
        }
    }
    let mut allowed_tools = Vec::new();
    for tool in requested.allowed_tools.iter().chain(&bound.allowed_tools) {
        if allows(requested, tool)
            && allows(bound, tool)
            && !disallowed_tools.contains(tool)
            && !allowed_tools.contains(tool)
        {
            allowed_tools.push(tool.clone());
        }
    }
    PermissionPolicy {
        preset: std::cmp::min_by_key(requested.preset, bound.preset, |preset| rank(*preset)),
        allowed_tools,
        disallowed_tools,
    }
}

impl AgentIndex {
    /// Ends a call with `reloading` when this index generation unloads; the
    /// `sessions` service and its callers stay (§8.3).
    async fn until_unloaded<T>(
        &self,
        call: impl Future<Output = Result<T, SessionsError>>,
    ) -> Result<T, SessionsError> {
        tokio::select! {
            biased;
            _ = self.stopping.cancelled() => Err(SessionsError::reloading("the Agents index")),
            result = call => result,
        }
    }

    /// Creates a non-interactive Session whose policy is the request capped
    /// by the starter branch's current policy, or by the host's Session
    /// defaults without a starter.
    pub async fn launch(
        self: &Arc<Self>,
        request: LaunchRequest,
    ) -> Result<LaunchReply, SessionsError> {
        self.until_unloaded(async {
            if request.policy.as_ref().is_some_and(|policy| {
                policy
                    .allowed_tools
                    .iter()
                    .chain(&policy.disallowed_tools)
                    .any(|name| name.trim().is_empty())
            }) {
                return Err(SessionsError::rejected("Tool names must not be empty"));
            }
            let host = self.host(request.agent.as_deref()).ok_or_else(|| {
                let id = request.agent.as_deref().unwrap_or(self.default_agent_id());
                match self.unavailable_agent(id) {
                    Some(reason) => SessionsError::new(SessionsErrorCode::Failed, reason),
                    None => SessionsError::new(
                        SessionsErrorCode::AgentUnknown,
                        format!("unknown agent: {id}"),
                    ),
                }
            })?;
            let cwd = session_cwd(&request.cwd)
                .await
                .map_err(|error| SessionsError::rejected(error.message))?;
            // Read once under a lease; a starter that cannot be read fails
            // the launch rather than falling back to a possibly wider bound.
            let bound = match &request.starter {
                Some(starter) => {
                    let lease = self.acquire(&starter.session_id).await?;
                    lease
                        .session
                        .get::<PermissionState>(&starter.branch_id)?
                        .ok_or_else(|| SessionsError::reloading("the starter's Permission"))?
                        .policy
                }
                None => self.session_defaults.permission_policy.clone(),
            };
            let requested = match request.policy {
                Some(policy) => policy,
                None if request.starter.is_some() => bound.clone(),
                None => PermissionPolicy::default(),
            };
            let policy = cap(&requested, &bound);
            let mut config = super::session_config(&host.profile, &self.session_defaults);
            config.cwd = cwd;
            config.permission_policy = policy.clone();
            config.interactive = false;
            if self.selects_models {
                let selection = self
                    .model_selection()
                    .ok_or_else(|| SessionsError::reloading("model selection"))?;
                config.model = selection
                    .0
                    .resolve(
                        request.model.or_else(|| host.profile.model.clone()),
                        Some(config.cwd.clone().into()),
                        self.stopping.clone(),
                    )
                    .await
                    .map_err(|error| {
                        SessionsError::new(
                            SessionsErrorCode::Failed,
                            format!("model resolution: {error}"),
                        )
                    })?;
            } else if let Some(model) = request.model {
                config.model = ModelSelection {
                    provider_id: model.provider_id,
                    model_id: model.model_id,
                    capability_snapshot: None,
                    config: None,
                    metadata: None,
                };
            }
            // Clients see a loaded Session once its protocol version is
            // registered: creation and registration finish together even
            // when the caller stops waiting.
            let index = self.clone();
            let session = tokio::spawn(async move {
                let session = index
                    .create_session(&host, config, PROTOCOL_VERSION)
                    .await?;
                index.retain_versions();
                index.insert_version(session.id(), &host.agent_id, PROTOCOL_VERSION);
                Ok::<_, AgentError>(session)
            })
            .await
            .map_err(|error| SessionsError::new(SessionsErrorCode::Failed, error.to_string()))?
            .map_err(|error| match error {
                AgentError::Reloading(_) => SessionsError::from(error),
                error => SessionsError::new(SessionsErrorCode::Failed, error.to_string()),
            })?;
            Ok(LaunchReply {
                session_id: session.id().to_owned(),
                policy,
            })
        })
        .await
    }
}

/// `sessions` by name for every language: each call decodes one request
/// object, calls the current index and replies with one envelope,
/// `{ok: true, value}` or `{ok: false, error: {code, message, retryable}}`.
pub(crate) struct SessionsService(pub(crate) Ctx);
impl HostDispatch for SessionsService {
    fn methods(&self) -> Option<serde_json::Value> {
        Some(json!({"launch": "async"}))
    }
    fn invoke(&self, method: &str, args: Value) -> Reply {
        // Looked up per call, so an index reload never reloads a caller.
        let index = self.0.get::<AgentIndex>();
        let method = method.to_owned();
        let args = args.json();
        Ok(Value::future(async move {
            let result = match (index, args) {
                (None, _) => Err(SessionsError::reloading("the Agents index")),
                (_, Err(error)) => Err(SessionsError::rejected(error.to_string())),
                (Some(index), Ok(args)) => call(&index, &method, args).await,
            };
            Ok(match result {
                Ok(value) => json!({"ok": true, "value": value}),
                Err(error) => json!({"ok": false, "error": {
                    "code": error.code,
                    "message": error.message,
                    "retryable": error.retryable(),
                }}),
            }
            .into())
        }))
    }
}

fn request<T: DeserializeOwned>(args: serde_json::Value) -> Result<T, SessionsError> {
    let serde_json::Value::Array(args) = args else {
        return Err(SessionsError::rejected("expected one request object"));
    };
    let Ok([request]) = <[serde_json::Value; 1]>::try_from(args) else {
        return Err(SessionsError::rejected("expected one request object"));
    };
    serde_json::from_value(request).map_err(|error| SessionsError::rejected(error.to_string()))
}
fn reply(value: impl Serialize) -> Result<serde_json::Value, SessionsError> {
    Ok(serde_json::to_value(value).expect("sessions replies are JSON data"))
}

async fn call(
    index: &Arc<AgentIndex>,
    method: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, SessionsError> {
    match method {
        "launch" => reply(index.launch(request(args)?).await?),
        _ => Err(SessionsError::rejected(format!(
            "unknown sessions method {method}"
        ))),
    }
}
