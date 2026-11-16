/// A model id without its namespace segment and tag, lowercased and split.
pub(crate) fn split_id(id: &str) -> Vec<String> {
    let lower = id.to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let name = name.rsplit_once(':').map_or(name, |(head, _)| head);
    split_tokens(name)
}

/// A display name, lowercased and split.
pub(crate) fn split_display(display: &str) -> Vec<String> {
    split_tokens(&display.to_lowercase())
}

pub(crate) fn split_tokens(name: &str) -> Vec<String> {
    name.split(['-', '_', '.', ' '])
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}
