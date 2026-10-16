//! Image blocks in a request the model cannot take become text, per request
//! (Attachments A.1). The adopted input and the history are never changed.
use base64::Engine;
use dim_agent_types::*;
use std::sync::Arc;

const NO_IMAGE_INPUT: &str = "[Image omitted: the current model does not accept images.]";
const UNSENDABLE: &str = "[Image omitted: the image cannot be sent to this model.]";

/// Image formats every adapter accepts (feature list).
const FORMATS: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Returns the request to send: the same request when every image is sent as
/// is, otherwise a copy whose unsendable images are text placeholders.
pub(super) fn adapt(request: &Arc<ModelRequest>) -> Arc<ModelRequest> {
    // Unknown capability: the image is sent and the provider decides (A4).
    let vision = request
        .capability_snapshot
        .as_ref()
        .is_none_or(|snapshot| snapshot.capabilities.vision);
    let replaced: Vec<_> = request
        .messages
        .iter()
        .map(|message| replace_images(message, vision))
        .collect();
    if replaced.iter().all(Option::is_none) {
        return request.clone();
    }
    let mut adapted = (**request).clone();
    adapted.messages = request
        .messages
        .iter()
        .zip(replaced)
        .map(|(message, replaced)| replaced.map_or_else(|| message.clone(), Arc::new))
        .collect();
    Arc::new(adapted)
}

/// The message with its unsendable images replaced, or `None` if it has none.
fn replace_images(message: &Message, vision: bool) -> Option<Message> {
    let MessageContent::Parts(parts) = &message.content else {
        return None;
    };
    let mut changed = false;
    let parts = parts
        .iter()
        .map(|part| {
            let MessagePart::Image { source } = part else {
                return part.clone();
            };
            let text = if !vision {
                NO_IMAGE_INPUT
            } else if !sendable(source) {
                UNSENDABLE
            } else {
                return part.clone();
            };
            changed = true;
            MessagePart::Text {
                provider_metadata: None,
                text: text.into(),
                start_time: None,
                end_time: None,
            }
        })
        .collect();
    changed.then(|| Message {
        role: message.role,
        content: MessageContent::Parts(parts),
    })
}

/// A URL source, undecodable data or a format adapters do not accept (A10).
fn sendable(source: &ImageSource) -> bool {
    source.source_type == ImageSourceType::Base64
        && FORMATS.contains(&source.media_type.as_str())
        && source.data.as_deref().is_some_and(|data| {
            !data.is_empty()
                && base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .is_ok()
        })
}
