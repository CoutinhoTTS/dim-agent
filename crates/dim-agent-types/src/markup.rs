//! Markup shared by the runtime's writers: the escape for an element
//! attribute or its text.

/// Escapes a value for one XML attribute or for element text, so text a
/// producer or a job wrote cannot open or close a tag.
pub fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::escaped;

    #[test]
    fn every_markup_character_is_escaped() {
        assert_eq!(
            escaped(r#"<a href="x">Tom & Jerry's</a>"#),
            "&lt;a href=&quot;x&quot;&gt;Tom &amp; Jerry&apos;s&lt;/a&gt;"
        );
    }
}
