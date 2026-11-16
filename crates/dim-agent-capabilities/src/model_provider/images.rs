//! Image blocks become content references per request (Attachments A.1).
//! The rule covers user content and tool results alike.
//! The adopted input and the history are never changed.
use crate::images::image_reference;
use dim_agent_types::*;
use std::sync::Arc;

const NO_IMAGE_INPUT: &str = "[Image omitted: the current model does not accept images.]";
const UNSENDABLE: &str = "[Image omitted: the image cannot be sent to this model.]";

/// Returns the request with image references or placeholders in place of
/// image blocks, sharing messages that contain no images.
pub(super) fn adapt(request: &Arc<ModelRequest>) -> Arc<ModelRequest> {
    let vision = request
        .capability_snapshot
        .as_ref()
        .is_none_or(|snapshot| snapshot.capabilities.vision);
    let replacements: Vec<_> = request
        .messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            replace_images(message, vision).map(|message| (index, Arc::new(message)))
        })
        .collect();
    if replacements.is_empty() {
        return request.clone();
    }
    Arc::new(ModelRequest {
        provider_id: request.provider_id.clone(),
        model_id: request.model_id.clone(),
        capability_snapshot: request.capability_snapshot.clone(),
        messages: replace_selected(&request.messages, replacements),
        tools: request.tools.clone(),
        system: request.system.clone(),
        config: request.config.clone(),
        metadata: request.metadata.clone(),
    })
}

/// The message with its images replaced, or `None` if it has none.
fn replace_images(message: &Message, vision: bool) -> Option<Message> {
    let MessageContent::Parts(parts) = &message.content else {
        return None;
    };
    let replacements: Vec<_> = parts
        .iter()
        .enumerate()
        .filter_map(|(index, part)| {
            let replacement = match part {
                MessagePart::Image { source } => Some(MessagePart::Text {
                    provider_metadata: None,
                    text: reference_or_placeholder(source, vision),
                    start_time: None,
                    end_time: None,
                }),
                MessagePart::ToolResult {
                    tool_use_id,
                    content: ToolContent::Blocks(blocks),
                    structured_content,
                    is_error,
                } => {
                    let replacements: Vec<_> = blocks
                        .iter()
                        .enumerate()
                        .filter_map(|(index, block)| {
                            let ContentBlock::Image { source } = block else {
                                return None;
                            };
                            Some((
                                index,
                                ContentBlock::Text {
                                    text: reference_or_placeholder(source, vision),
                                },
                            ))
                        })
                        .collect();
                    (!replacements.is_empty()).then(|| MessagePart::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: ToolContent::Blocks(replace_selected(blocks, replacements)),
                        structured_content: structured_content.clone(),
                        is_error: *is_error,
                    })
                }
                _ => None,
            };
            replacement.map(|part| (index, part))
        })
        .collect();
    (!replacements.is_empty()).then(|| Message {
        role: message.role,
        content: MessageContent::Parts(replace_selected(parts, replacements)),
    })
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

fn reference_or_placeholder(source: &ImageSource, vision: bool) -> String {
    image_reference(source)
        .unwrap_or_else(|| if vision { UNSENDABLE } else { NO_IMAGE_INPUT }.into())
}
