//! Local input both clients attach to a message: the limits a draft keeps
//! to, and the files a paste names.

use std::path::{Path, PathBuf};

/// A paste longer than this is stored as a session attachment
/// (`attachment.put`) instead of being sent inline.
pub const INLINE_PASTE: usize = 16 * 1024;
/// Longest paste a draft accepts.
pub const MAX_PASTE: usize = 10 * 1024 * 1024;
/// Images one submission accepts.
pub const MAX_IMAGES: usize = 20;

/// The local path a `file:` URL names, if it is local.
pub fn file_url(value: &str) -> Option<PathBuf> {
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != "file" || !matches!(url.host_str(), None | Some("localhost")) {
        return None;
    }
    url.to_file_path().ok()
}

fn local_path(value: &str, base: &Path) -> Option<PathBuf> {
    if value.starts_with("file:") {
        file_url(value)
    } else if value.starts_with("http://") || value.starts_with("https://") {
        None
    } else {
        Some(base.join(value))
    }
}

/// The local files a paste names: one path, or several quoted or
/// backslash-escaped ones, `file:` URLs included; `None` unless every one
/// is an existing file.
pub fn pasted_paths(text: &str, base: &Path) -> Option<Vec<PathBuf>> {
    let value = text.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(path) = local_path(value, base).filter(|path| path.is_file()) {
        return Some(vec![path]);
    }
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if chars
                .peek()
                .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '\'' | '"')) =>
            {
                token.push(chars.next()?)
            }
            '\'' | '"' if quote == Some(ch) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(ch),
            ch if ch.is_whitespace() && quote.is_none() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            ch => token.push(ch),
        }
    }
    if quote.is_some() {
        return None;
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    if tokens.is_empty() {
        return None;
    }
    tokens
        .iter()
        .map(|token| local_path(token, base).filter(|path| path.is_file()))
        .collect()
}

