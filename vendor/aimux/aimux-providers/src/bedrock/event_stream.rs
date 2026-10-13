//! Minimal AWS Event Stream encoder/decoder.
//!
//! Amazon Bedrock's `converse-stream` endpoint returns responses in the AWS
//! event stream binary format (`application/vnd.amazon.eventstream`). Each
//! message is a framed binary record containing headers (notably
//! `:message-type` and `:event-type`) and a JSON payload.
//!
//! This module implements just enough of the format to decode Bedrock stream
//! events and to encode them in tests.
//!
//! Reference: <https://docs.aws.amazon.com/transcribe/latest/dg/event-stream.md>
//!            <https://github.com/smithy-lang/smithy-typescript/tree/main/packages/eventstream-codec>

/// CRC32 lookup table (IEEE polynomial 0xEDB88320).
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFFFFFFu32;
    for &byte in data {
        crc = (crc >> 8) ^ CRC32_TABLE[((crc ^ byte as u32) & 0xFF) as usize];
    }
    crc ^ 0xFFFFFFFF
}

/// A decoded event stream message.
#[derive(Debug, Clone)]
pub struct EventStreamMessage {
    /// The `:message-type` header value (typically `"event"`).
    pub message_type: String,
    /// The `:event-type` header value (e.g. `"contentBlockDelta"`).
    pub event_type: String,
    /// The message payload as a UTF-8 string (typically JSON).
    pub data: String,
}

/// Header value types in the AWS event stream format.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum HeaderValue {
    String(String),
    Bool(bool),
}

/// Encode a single event stream message.
///
/// Returns the binary frame ready to be sent on the wire.
#[must_use]
pub fn encode_message(message_type: &str, event_type: &str, payload: &str) -> Vec<u8> {
    // Headers: :message-type, :event-type, :content-type
    let headers = vec![
        (
            ":message-type",
            HeaderValue::String(message_type.to_string()),
        ),
        (":event-type", HeaderValue::String(event_type.to_string())),
        (
            ":content-type",
            HeaderValue::String("application/json".to_string()),
        ),
    ];

    let mut header_buf = Vec::new();

    for (name, value) in &headers {
        // name length (1 byte) + name + type (1 byte) + value
        header_buf.push(name.len() as u8);
        header_buf.extend_from_slice(name.as_bytes());
        match value {
            HeaderValue::String(s) => {
                header_buf.push(7); // type 7 = string
                header_buf.extend_from_slice(&(s.len() as u16).to_be_bytes());
                header_buf.extend_from_slice(s.as_bytes());
            }
            HeaderValue::Bool(b) => {
                header_buf.push(if *b { 0 } else { 1 }); // type 0/1 = bool true/false
            }
        }
    }

    let payload_bytes = payload.as_bytes();

    // Prelude: total_length(4) + headers_length(4) + prelude_crc(4)
    // Then headers, then payload, then message_crc(4)
    let total_length = (4 + 4 + 4 + header_buf.len() + payload_bytes.len() + 4) as u32;
    let headers_length = header_buf.len() as u32;

    // Prelude CRC: CRC32 of the first 8 bytes (total_length + headers_length)
    let mut prelude = Vec::new();
    prelude.extend_from_slice(&total_length.to_be_bytes());
    prelude.extend_from_slice(&headers_length.to_be_bytes());
    let prelude_crc = crc32(&prelude);

    let mut message = Vec::new();
    message.extend_from_slice(&total_length.to_be_bytes());
    message.extend_from_slice(&headers_length.to_be_bytes());
    message.extend_from_slice(&prelude_crc.to_be_bytes());
    message.extend_from_slice(&header_buf);
    message.extend_from_slice(payload_bytes);
    let msg_crc = crc32(&message);
    message.extend_from_slice(&msg_crc.to_be_bytes());

    message
}

/// Encode multiple messages into a single byte stream.
#[must_use]
pub fn encode_messages(messages: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (msg_type, event_type, payload) in messages {
        buf.extend_from_slice(&encode_message(msg_type, event_type, payload));
    }
    buf
}

/// Incremental, CRC-validating decoder backed by the official Smithy codec.
pub struct EventStreamDecoder {
    decoder: aws_smithy_eventstream::frame::MessageFrameDecoder,
    buffer: bytes::BytesMut,
    pending: bool,
}

impl Default for EventStreamDecoder {
    fn default() -> Self {
        Self {
            decoder: aws_smithy_eventstream::frame::MessageFrameDecoder::new(),
            buffer: bytes::BytesMut::new(),
            pending: false,
        }
    }
}

impl EventStreamDecoder {
    const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

    pub fn push(&mut self, data: &[u8]) -> Result<Vec<EventStreamMessage>, String> {
        if self.buffer.len().saturating_add(data.len()) > Self::MAX_FRAME_SIZE {
            return Err(format!(
                "Bedrock event stream frame exceeds {} bytes",
                Self::MAX_FRAME_SIZE
            ));
        }
        self.buffer.extend_from_slice(data);
        let mut messages = Vec::new();
        loop {
            if self.buffer.is_empty() && !self.pending {
                break;
            }
            // The decoder may consume a complete prelude and retain it internally.
            // An empty external buffer is then still an unfinished frame.
            self.pending = true;
            match self
                .decoder
                .decode_frame(&mut self.buffer)
                .map_err(|error| error.to_string())?
            {
                aws_smithy_eventstream::frame::DecodedFrame::Incomplete => {
                    break;
                }
                aws_smithy_eventstream::frame::DecodedFrame::Complete(message) => {
                    self.pending = false;
                    let mut message_type = None;
                    let mut event_type = None;
                    for header in message.headers() {
                        let target = match header.name().as_str() {
                            ":message-type" => &mut message_type,
                            ":event-type" | ":exception-type" => &mut event_type,
                            _ => continue,
                        };
                        if target.is_some() {
                            return Err("duplicate Bedrock event header".into());
                        }
                        let value = header
                            .value()
                            .as_string()
                            .map_err(|_| "Bedrock event header must be a string")?
                            .as_str();
                        *target = Some(value.to_string());
                    }
                    messages.push(EventStreamMessage {
                        message_type: message_type.ok_or("Bedrock event missing :message-type")?,
                        event_type: event_type.ok_or("Bedrock event missing event type")?,
                        data: std::str::from_utf8(message.payload().as_ref())
                            .map_err(|error| error.to_string())?
                            .to_string(),
                    });
                }
            }
        }
        Ok(messages)
    }

    pub fn finish(&self) -> Result<(), String> {
        if self.pending || !self.buffer.is_empty() {
            Err("truncated Bedrock event stream frame".into())
        } else {
            Ok(())
        }
    }
}

pub fn decode_messages(data: &[u8]) -> Result<Vec<EventStreamMessage>, String> {
    let mut decoder = EventStreamDecoder::default();
    let messages = decoder.push(data)?;
    decoder.finish()?;
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_single_message() {
        let payload = r#"{"contentBlockIndex":0,"delta":{"text":"Hello"}}"#;
        let encoded = encode_message("event", "contentBlockDelta", payload);
        let decoded = decode_messages(&encoded).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].message_type, "event");
        assert_eq!(decoded[0].event_type, "contentBlockDelta");
        assert_eq!(decoded[0].data, payload);
    }

    #[test]
    fn round_trip_multiple_messages() {
        let messages = vec![
            ("event", "messageStart", r#"{"role":"assistant""#),
            (
                "event",
                "contentBlockDelta",
                r#"{"contentBlockIndex":0,"delta":{"text":"Hi"}}"#,
            ),
            ("event", "messageStop", r#"{"stopReason":"end_turn"}"#),
        ];
        let encoded = encode_messages(&messages);
        let decoded = decode_messages(&encoded).unwrap();
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].event_type, "messageStart");
        assert_eq!(decoded[1].event_type, "contentBlockDelta");
        assert_eq!(decoded[2].event_type, "messageStop");
    }

    #[test]
    fn corrupt_prelude_crc_is_rejected() {
        let payload = r#"{"delta":{"text":"Hi"}}"#;
        let mut encoded = encode_message("event", "contentBlockDelta", payload);
        // Flip a byte in the prelude CRC (bytes 8..12).
        encoded[8] ^= 0xFF;
        assert!(decode_messages(&encoded).is_err());
    }

    #[test]
    fn corrupt_message_crc_is_rejected() {
        let payload = r#"{"delta":{"text":"Hi"}}"#;
        let mut encoded = encode_message("event", "contentBlockDelta", payload);
        // Flip the last byte (part of the trailing message CRC).
        let last = encoded.len() - 1;
        encoded[last] ^= 0xFF;
        assert!(decode_messages(&encoded).is_err());
    }

    #[test]
    fn corrupt_payload_is_rejected_by_crc() {
        let payload = r#"{"delta":{"text":"Hi"}}"#;
        let mut encoded = encode_message("event", "contentBlockDelta", payload);
        // Flip a byte in the payload region (middle of the frame).
        let mid = encoded.len() / 2;
        encoded[mid] ^= 0xFF;
        assert!(decode_messages(&encoded).is_err());
    }

    #[test]
    fn valid_frame_before_corrupt_one_is_kept() {
        // Two frames: first valid, second with a flipped payload byte.
        let good = encode_message("event", "messageStart", r#"{"role":"assistant"}"#);
        let mut bad = encode_message("event", "contentBlockDelta", r#"{"delta":{"text":"x"}}"#);
        let bad_mid = bad.len() / 2;
        bad[bad_mid] ^= 0xFF;
        let mut buf = good.clone();
        buf.extend_from_slice(&bad);
        assert!(decode_messages(&buf).is_err());
    }

    #[test]
    fn split_frame_decodes_and_truncation_is_rejected() {
        let encoded = encode_message("event", "messageStart", r#"{"role":"assistant"}"#);
        let mut decoder = EventStreamDecoder::default();
        let split = encoded.len() / 2;
        assert!(decoder.push(&encoded[..split]).unwrap().is_empty());
        let messages = decoder.push(&encoded[split..]).unwrap();
        assert_eq!(messages.len(), 1);
        decoder.finish().unwrap();

        let mut truncated = EventStreamDecoder::default();
        truncated.push(&encoded[..encoded.len() - 1]).unwrap();
        assert!(truncated.finish().is_err());
    }
}
