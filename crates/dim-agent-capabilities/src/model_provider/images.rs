//! Image blocks in a request the model cannot take become content references
//! per request (Attachments A.1). The rule covers user content and tool
//! results alike. A vision-capable model receives a sendable image as it is.
//! The adopted input and the history are never changed.
use crate::host::{HostError, SessionFiles};
use crate::images::image_reference;
use base64::Engine;
use dim_agent_types::*;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const NO_IMAGE_INPUT: &str = "[Image omitted: the current model does not accept images.]";
const UNSENDABLE: &str = "[Image omitted: the image cannot be sent to this model.]";

/// Image formats every adapter accepts (feature list).
const FORMATS: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// What one image becomes in the request copy.
enum Routed {
    /// The source is already the form this model takes.
    Keep,
    /// The file-backed source resolved to the bytes this model takes.
    Resolved(ImageSource),
    /// The reference or placeholder that replaces the image.
    Text(String),
}

/// Returns the request to send: the same request when every image is sent as
/// is, otherwise a copy whose images a model cannot take become references or
/// placeholders. Messages without images stay shared.
pub(super) async fn adapt(
    request: &Arc<ModelRequest>,
    files: Option<&SessionFiles>,
    cancel: &CancellationToken,
) -> Result<Arc<ModelRequest>, HostError> {
    // Unknown capability: the image is sent and the provider decides (A4).
    let vision = request
        .capability_snapshot
        .as_ref()
        .is_none_or(|snapshot| snapshot.capabilities.vision);
    let mut replacements = Vec::new();
    for (index, message) in request.messages.iter().enumerate() {
        if let Some(message) = replace_images(message, request, files, vision, cancel).await? {
            replacements.push((index, Arc::new(message)));
        }
    }
    if replacements.is_empty() {
        return Ok(request.clone());
    }
    Ok(Arc::new(ModelRequest {
        provider_id: request.provider_id.clone(),
        model_id: request.model_id.clone(),
        capability_snapshot: request.capability_snapshot.clone(),
        messages: replace_selected(&request.messages, replacements),
        tools: request.tools.clone(),
        system: request.system.clone(),
        config: request.config.clone(),
        metadata: request.metadata.clone(),
    }))
}

/// The message with its images routed, or `None` if every image is sent as it
/// is.
async fn replace_images(
    message: &Message,
    request: &ModelRequest,
    files: Option<&SessionFiles>,
    vision: bool,
    cancel: &CancellationToken,
) -> Result<Option<Message>, HostError> {
    let MessageContent::Parts(parts) = &message.content else {
        return Ok(None);
    };
    let mut replacements = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let replacement = match part {
            MessagePart::Image { source } => {
                match route(source, request, files, vision, cancel).await? {
                    Routed::Keep => None,
                    Routed::Resolved(source) => Some(MessagePart::Image { source }),
                    Routed::Text(text) => Some(MessagePart::Text {
                        provider_metadata: None,
                        text,
                        start_time: None,
                        end_time: None,
                    }),
                }
            }
            MessagePart::ToolResult {
                tool_use_id,
                content: ToolContent::Blocks(blocks),
                structured_content,
                is_error,
            } => {
                let mut replacements = Vec::new();
                for (index, block) in blocks.iter().enumerate() {
                    let ContentBlock::Image { source } = block else {
                        continue;
                    };
                    match route(source, request, files, vision, cancel).await? {
                        Routed::Keep => {}
                        Routed::Resolved(source) => {
                            replacements.push((index, ContentBlock::Image { source }));
                        }
                        Routed::Text(text) => {
                            replacements.push((index, ContentBlock::Text { text }));
                        }
                    }
                }
                (!replacements.is_empty()).then(|| MessagePart::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: ToolContent::Blocks(replace_selected(blocks, replacements)),
                    structured_content: structured_content.clone(),
                    is_error: *is_error,
                })
            }
            _ => None,
        };
        if let Some(part) = replacement {
            replacements.push((index, part));
        }
    }
    Ok((!replacements.is_empty()).then(|| Message {
        role: message.role,
        content: MessageContent::Parts(replace_selected(parts, replacements)),
    }))
}

fn replace_selected<T: Clone>(values: &[T], replacements: Vec<(usize, T)>) -> Vec<T> {
    let mut replacements = replacements.into_iter().peekable();
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if replacements.peek().is_some_and(|(at, _)| *at == index) {
                replacements.next().unwrap().1
            } else {
                value.clone()
            }
        })
        .collect()
}

/// The form one image takes for this model: sent as it is when the model
/// takes images and the image is sendable, else its reference or placeholder.
async fn route(
    source: &ImageSource,
    request: &ModelRequest,
    files: Option<&SessionFiles>,
    vision: bool,
    cancel: &CancellationToken,
) -> Result<Routed, HostError> {
    crate::context_policy::check_cancel(cancel)?;
    let resolvable = source.source_type == ImageSourceType::File && sendable(source);
    let resolved;
    let source = if resolvable {
        resolved = crate::session_files::resolve(request, files, source, cancel).await?;
        &resolved
    } else {
        source
    };
    if vision && (resolvable || sendable(source)) {
        return Ok(if resolvable {
            Routed::Resolved(source.clone())
        } else {
            Routed::Keep
        });
    }
    Ok(Routed::Text(image_reference(source).unwrap_or_else(|| {
        if vision { UNSENDABLE } else { NO_IMAGE_INPUT }.into()
    })))
}

/// A URL source, a file without an id, undecodable data or a format adapters
/// do not accept (A10).
fn sendable(source: &ImageSource) -> bool {
    FORMATS.contains(&source.media_type.as_str())
        && match source.source_type {
            ImageSourceType::File => source.file_id.as_ref().is_some_and(|id| !id.is_empty()),
            ImageSourceType::Base64 => source.data.as_deref().is_some_and(|data| {
                !data.is_empty()
                    && base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .is_ok()
            }),
            ImageSourceType::Url => false,
        }
}
