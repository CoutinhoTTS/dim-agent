use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Per-call tool names use the same encoding and hash recovery as the Node driver.
#[derive(Default)]
pub(crate) struct ToolNames {
    wire: HashMap<String, String>,
    original: HashMap<String, String>,
    hashes: HashMap<String, Option<String>>,
}

impl ToolNames {
    pub(super) fn register(&mut self, original: &str) -> Result<String, String> {
        if let Some(wire) = self.wire.get(original) {
            return Ok(wire.clone());
        }
        if original.trim().is_empty() {
            return Err("tool name must not be empty".into());
        }
        let wire = encode(original);
        if let Some(existing) = self.original.get(&wire)
            && existing != original
        {
            return Err(format!(
                "tool names {existing:?} and {original:?} both map to {wire:?}"
            ));
        }
        if let Some(hash) = generated_hash(&wire) {
            self.hashes
                .entry(hash.to_owned())
                .and_modify(|entry| *entry = None)
                .or_insert_with(|| Some(original.to_owned()));
        }
        self.original.insert(wire.clone(), original.to_owned());
        self.wire.insert(original.to_owned(), wire.clone());
        Ok(wire)
    }

    pub(crate) fn original<'a>(&'a self, wire_name: &'a str) -> &'a str {
        if let Some(original) = self.original.get(wire_name) {
            return original;
        }
        generated_hash(wire_name)
            .and_then(|hash| self.hashes.get(hash))
            .and_then(Option::as_deref)
            .unwrap_or(wire_name)
    }
}

fn allowed(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn generated_hash(name: &str) -> Option<&str> {
    let (prefix, hash) = name.rsplit_once('_')?;
    (!prefix.is_empty()
        && prefix.bytes().all(allowed)
        && hash.len() == 12
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(hash)
}

fn encode(original: &str) -> String {
    if original.len() <= 64 && original.bytes().all(allowed) {
        return original.to_owned();
    }
    let hash = format!("{:x}", Sha256::digest(original.as_bytes()));
    let tail = original.split_once("__").map_or(original, |(_, tail)| tail);
    let readable = if !tail.is_empty() && tail.bytes().all(allowed) {
        tail.to_owned()
    } else {
        let mut readable = String::new();
        for character in tail.chars() {
            if character.is_ascii() && allowed(character as u8) && character != '_' {
                readable.push(character);
            } else if !readable.ends_with('_') {
                readable.push('_');
            }
        }
        let readable = readable.trim_matches('_');
        if readable.is_empty() {
            "tool"
        } else {
            readable
        }
        .to_owned()
    };
    format!("{}_{}", &readable[..readable.len().min(51)], &hash[..12])
}
