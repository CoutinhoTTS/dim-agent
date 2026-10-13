//! Text of runtime blocks. Producers build the final text of a raw input
//! block with these helpers so tags and escaping stay uniform; the history
//! renderer places that text verbatim.
use dim_agent_types::*;

/// A raw input block carrying `text`, optionally in effect under `key`.
pub fn raw(text: String, key: Option<String>) -> InputBlock {
    InputBlock::Raw {
        text,
        key,
        retire: false,
    }
}

/// A raw input block delivering `text` and ending `key`.
pub fn retire(text: String, key: String) -> InputBlock {
    InputBlock::Raw {
        text,
        key: Some(key),
        retire: true,
    }
}

/// Escapes text placed inside a runtime tag.
pub fn text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escapes an attribute value of a runtime tag.
pub fn attr(value: &str) -> String {
    text(value).replace('"', "&quot;")
}

fn open(name: &str, attrs: &[(&str, &str)]) -> String {
    let mut tag = format!("<{name}");
    for (key, value) in attrs {
        tag.push_str(&format!(" {key}=\"{}\"", attr(value)));
    }
    tag.push('>');
    tag
}

/// `<name attrs>` on its own line, then `body`, then `</name>`. `body` is
/// already escaped.
pub fn element(name: &str, attrs: &[(&str, &str)], body: &str) -> String {
    format!("{}\n{body}\n</{name}>", open(name, attrs))
}

/// Escaped segments joined by newlines, cut at 16 KiB of escaped text.
pub fn preview(segments: &[&str]) -> String {
    const LIMIT: usize = 16 * 1024;
    const TRUNCATED: &str = "\n[Preview truncated at 16 KiB.]";

    let escaped = text(&segments.join("\n"));
    if escaped.len() <= LIMIT {
        return escaped;
    }
    let mut end = LIMIT - TRUNCATED.len();
    while !escaped.is_char_boundary(end) {
        end -= 1;
    }
    // Never cut inside an entity.
    if let Some(amp) = escaped[..end].rfind('&')
        && !escaped[amp..end].contains(';')
    {
        end = amp;
    }
    format!("{}{TRUNCATED}", &escaped[..end])
}
