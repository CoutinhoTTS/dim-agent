//! Attachments ([design](../../../docs/design/plugins/Attachments.md)): the
//! Session method `attachment.put`, which stores a large paste as a Session
//! file through SessionStore's `sessionFiles` and returns its path. The model
//! reads the file with `read` like any file; images need no plugin.
use dim_agent_capabilities::{
    host::{SessionFileError, SessionFileErrorCode, SessionFilesHost},
    methods::{self, Interruption, MethodDecl, RpcError, SessionMethods},
};
use dim_agent_runtime::{AgentBuilder, SessionIdentity};
use rutis::{BoxFuture, CordisError, Ctx, Effect, Keyed, TypedPlugin};
use std::{path::PathBuf, sync::Arc};

/// The `sessionMethods` namespace Attachments provides.
const NAMESPACE: &str = "attachment";
/// The content limit of one put (A2); SessionStore's own cap is 16 MiB.
const MAX_TEXT_BYTES: usize = 10 * 1024 * 1024;

/// Records the namespace and installs the Session part. A host calls this
/// only where SessionStore provides `sessionFiles` and the model has `read`,
/// before Rpc reads the recorded namespaces.
pub fn install(builder: &mut AgentBuilder) {
    builder.session_part(|names| names.part(AttachmentsPlugin));
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PutParams {
    /// Read by Rpc to route the call; the provider stores for its own Session.
    #[serde(rename = "sessionId")]
    _session_id: String,
    name: Option<String>,
    text: String,
}

/// SessionStore's reply passed through, with `attachmentId = fileId`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PutReply {
    attachment_id: String,
    path: PathBuf,
    bytes: u64,
}

/// `attachment.put` errors for SessionStore's codes (design A.2).
fn put_error(error: SessionFileError) -> RpcError {
    let code = match error.code {
        SessionFileErrorCode::NotFound => return RpcError::closed(),
        // Nothing was stored, so the client may retry.
        SessionFileErrorCode::Cancelled => return RpcError::reloading(),
        SessionFileErrorCode::Invalid => "attachment_invalid",
        SessionFileErrorCode::TooLarge => "attachment_too_large",
        SessionFileErrorCode::NodeUnavailable => "node_unavailable",
        SessionFileErrorCode::Storage => "storage_failed",
    };
    RpcError::app(-32000, code, error.message)
}

pub struct AttachmentsPlugin;
impl TypedPlugin for AttachmentsPlugin {
    type Deps = (
        Keyed<SessionIdentity>,
        Keyed<SessionFilesHost>,
        Keyed<SessionMethods>,
    );
    fn name(&self) -> &str {
        "Attachments"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(identity), Keyed(files), Keyed(methods)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let session_id = identity.session_id.clone();
            // `drain`: an unload waits for the put either way, and the client
            // gets the path or `service_reloading` with nothing stored.
            methods.register(
                ctx,
                NAMESPACE,
                vec![MethodDecl::new("attachment.put", true, Interruption::Drain)],
                Arc::new(move |_, value, cancel, _| {
                    let (files, session_id) = (files.clone(), session_id.clone());
                    Box::pin(async move {
                        let PutParams { name, text, .. } = methods::params(value)?;
                        if text.is_empty() {
                            return Err(RpcError::app(
                                -32000,
                                "attachment_invalid",
                                "Attachment text is empty",
                            ));
                        }
                        if text.len() > MAX_TEXT_BYTES {
                            return Err(RpcError::app(
                                -32000,
                                "attachment_too_large",
                                "Attachment text exceeds 10 MiB",
                            ));
                        }
                        let stored = files
                            .0
                            .put(
                                session_id,
                                name.unwrap_or_else(|| "paste.txt".into()),
                                text.into_bytes(),
                                cancel,
                            )
                            .await
                            .map_err(put_error)?;
                        methods::reply(PutReply {
                            attachment_id: stored.file_id,
                            path: stored.path,
                            bytes: stored.bytes,
                        })
                    })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}
