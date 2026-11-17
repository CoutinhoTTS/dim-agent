//! `check` fails when a comment links a document under `docs/` that is missing,
//! or an anchor that is not a heading of it. A comment link resolves against
//! the source file's own directory, so a file that moves leaves its inbound
//! links broken: rustdoc resolves intra-doc item links only, and the checks
//! under `docs/` do not read Rust sources.
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Component, Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const SOURCE_ROOTS: [&str; 3] = ["crates", "xtask", "tests"];
const SKIPPED: [&str; 3] = ["target", "vendor", "node_modules"];

/// Checks every comment link under `root`. Each failure names the file, the
/// line, the link and why it fails.
pub fn check(root: &Path) -> Result<()> {
    let mut errors = Vec::new();
    let mut checked = 0;
    for path in sources(root) {
        let text = fs::read_to_string(&path)?;
        for (line, comment) in comments(&text) {
            for target in link_targets(&comment) {
                let Some(link) = docs_link(&path, root, target) else {
                    continue;
                };
                checked += 1;
                if let Some(reason) = failure(&link) {
                    errors.push(format!(
                        "{}:{line}: `{target}`: {reason}",
                        path.strip_prefix(root).unwrap_or(&path).display()
                    ));
                }
            }
        }
    }
    if errors.is_empty() {
        println!("comment links passed ({checked} links into docs)");
        Ok(())
    } else {
        Err(errors.join("\n").into())
    }
}

/// A link in a comment whose path is a document under `docs/`.
struct DocsLink {
    file: PathBuf,
    anchor: Option<String>,
}

fn sources(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root_name in SOURCE_ROOTS {
        let directory = root.join(root_name);
        if directory.is_dir() {
            sources_in(&directory, &mut files);
        }
    }
    files.sort();
    files
}

fn sources_in(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("a source directory reads") {
        let path = entry.expect("a source entry reads").path();
        if SKIPPED.contains(&path_name(&path).as_str()) {
            continue;
        }
        if path.is_dir() {
            sources_in(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn path_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Each comment in `text`, as its first line number and its text with the
/// `//`, `/*` and `*/` delimiters removed. A comment is either a `//` line or a
/// `/* */` block, and a block does not nest. Neither opens inside a string or a
/// character literal, where the same characters mean something else: a glob
/// such as `"**/*.rs"` is not a comment start.
fn comments(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut rest = text;
    let mut line = 1;
    while let Some(character) = rest.chars().next() {
        let skipped = if let Some(body) = rest.strip_prefix("//") {
            let end = body.find('\n').unwrap_or(body.len());
            found.push((line, body[..end].to_owned()));
            2 + end
        } else if let Some(body) = rest.strip_prefix("/*") {
            let end = body.find("*/").unwrap_or(body.len());
            found.push((line, body[..end].to_owned()));
            2 + end + if end < body.len() { 2 } else { 0 }
        } else if let Some(length) = literal(rest) {
            length
        } else {
            character.len_utf8()
        };
        line += rest[..skipped].matches('\n').count();
        rest = &rest[skipped..];
    }
    found
}

/// The length of the string or character literal that starts `rest`, `None`
/// when the first character starts neither. The `b` and `b`+`r` prefixes are
/// left to the plain and raw string arms, which then begin at the quote.
fn literal(rest: &str) -> Option<usize> {
    if let Some(body) = rest.strip_prefix('"') {
        return Some(1 + quoted(body, '"'));
    }
    if let Some(body) = rest.strip_prefix('\'') {
        return character_literal(body).map(|length| 1 + length);
    }
    let body = rest.strip_prefix('r')?;
    let hashes = body.len() - body.trim_start_matches('#').len();
    let body = body[hashes..].strip_prefix('"')?;
    let terminator = format!("\"{}", "#".repeat(hashes));
    let end = body.find(&terminator).unwrap_or(body.len());
    Some(
        1 + hashes
            + 1
            + end
            + if end < body.len() {
                terminator.len()
            } else {
                0
            },
    )
}

/// The bytes from the start of `body` through its closing `quote`, escaped
/// characters skipped: the whole of `body` when it never closes.
fn quoted(body: &str, quote: char) -> usize {
    let mut characters = body.char_indices();
    while let Some((offset, character)) = characters.next() {
        if character == '\\' {
            characters.next();
        } else if character == quote {
            return offset + 1;
        }
    }
    body.len()
}

/// The bytes from the start of `body` through the closing quote of the
/// character literal whose opening quote precedes it, or `None` for a lifetime
/// such as `'a`, which has no closing quote of its own.
fn character_literal(body: &str) -> Option<usize> {
    let (_, character) = body.char_indices().next()?;
    if character == '\\' {
        return Some(quoted(body, '\''));
    }
    let end = character.len_utf8();
    body[end..].starts_with('\'').then_some(end + 1)
}

/// The targets of the Markdown links in `comment`, in source order: inline
/// `[text](target)` links and `[label]: target` reference definitions alike, so
/// a definition that moves with its document is checked like the reference that
/// uses it.
fn link_targets(comment: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut rest = comment;
    while let Some(open) = rest.find("](") {
        rest = &rest[open + 2..];
        let Some(close) = rest.find(')') else { break };
        push_target(&rest[..close], &mut targets);
        rest = &rest[close + 1..];
    }
    for line in comment.lines() {
        let line = line.trim_start_matches([' ', '\t', '*', '/', '!']);
        if let Some((_, target)) = line
            .strip_prefix('[')
            .and_then(|text| text.split_once("]:"))
        {
            push_target(target, &mut targets);
        }
    }
    targets
}

/// Pushes the link target in `text`, `None` for nothing: Markdown lets either
/// form wrap the path in angle brackets and follow it with a title.
fn push_target<'a>(text: &'a str, targets: &mut Vec<&'a str>) {
    let target = text.trim();
    let target = match target.strip_prefix('<') {
        Some(rest) => rest.split_once('>').map_or(rest, |(path, _)| path),
        None => target.split_whitespace().next().unwrap_or_default(),
    };
    if !target.is_empty() {
        targets.push(target);
    }
}

/// The link as a document under `docs/`, when it names one. A link to a URL, to
/// a path outside `docs/`, or an intra-doc item link is not checked, and
/// whether it names `docs/` is read from the path as written: a link one level
/// short still names `docs/`, which is the whole reason it is checked.
fn docs_link(source: &Path, root: &Path, target: &str) -> Option<DocsLink> {
    let (path, anchor) = match target.split_once('#') {
        Some((path, anchor)) => (path, Some(anchor.to_owned())),
        None => (target, None),
    };
    if path.is_empty()
        || !path.contains("docs/")
        || path.starts_with('/')
        || path.starts_with("http://")
        || path.starts_with("https://")
        || path.starts_with("mailto:")
        || path.contains("::")
    {
        return None;
    }
    let directory = source.parent().unwrap_or(root);
    Some(DocsLink {
        file: resolve(&directory.join(path)),
        anchor,
    })
}

/// `path` with its `.` and `..` components resolved, without requiring the
/// path to exist.
fn resolve(path: &Path) -> PathBuf {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                parts.pop();
            }
            Component::CurDir => {}
            other => parts.push(other.as_os_str()),
        }
    }
    parts.iter().collect()
}

fn failure(link: &DocsLink) -> Option<String> {
    let text = match fs::read_to_string(&link.file) {
        Ok(text) => text,
        Err(_) => return Some("no such file".to_owned()),
    };
    let anchor = link.anchor.as_deref()?;
    if headings(&text).contains(anchor) {
        return None;
    }
    Some(format!("no heading `#{anchor}` in the document"))
}

/// The GitHub heading slugs of a Markdown document. A repeated heading also
/// gets `-1`, `-2` variants, which is what a link to a duplicate must name.
fn headings(text: &str) -> BTreeSet<String> {
    let mut slugs = BTreeSet::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.starts_with("```") || line.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let Some(rest) = line.trim_start().strip_prefix('#') else {
            continue;
        };
        let heading = rest.trim_start_matches('#').trim();
        if heading.is_empty() {
            continue;
        }
        let slug = slug(heading);
        let count = seen.entry(slug.clone()).or_insert(0);
        slugs.insert(if *count == 0 {
            slug
        } else {
            format!("{slug}-{count}")
        });
        *count += 1;
    }
    slugs
}

/// GitHub's heading slug: lowercase, code spans, emphasis and link markup
/// dropped, non-ASCII letters kept, spaces to `-`.
fn slug(heading: &str) -> String {
    let mut text = heading.to_lowercase();
    while let Some(open) = text.find("](") {
        match text[open..].find(')') {
            Some(close) => {
                text.replace_range(open..open + close + 1, "");
            }
            None => break,
        }
    }
    text.retain(|character| !matches!(character, '`' | '[' | ']' | '<' | '>' | '*' | '!'));
    text.chars()
        .filter(|character| character.is_alphanumeric() || matches!(character, '-' | ' ' | '_'))
        .collect::<String>()
        .trim()
        .replace(' ', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_comment_and_a_line_comment_are_both_read() {
        let text = "let x = 1;\n/*one\ntwo*/ let y = 2;\n// three\n";
        assert_eq!(
            comments(text),
            vec![(2, "one\ntwo".to_owned()), (4, " three".to_owned())]
        );
    }

    #[test]
    fn a_link_after_a_glob_string_literal_keeps_its_own_line() {
        let text = concat!(
            "let pattern = \"**/*.rs\";\n",
            "let quote = '\"';\n",
            "let lifetime: &'a str = r#\"**/*.ts\"#;\n",
            "/// [Prompt §6.1]: ../../../docs/design/plugins/Prompt.md#61-the-plugin-bridge\n",
        );
        let link = comments(text)
            .into_iter()
            .filter_map(|(line, comment)| {
                link_targets(&comment)
                    .into_iter()
                    .next()
                    .map(|target| (line, target.to_owned()))
            })
            .next()
            .expect("the doc line is a comment");
        assert_eq!(
            link,
            (
                4,
                "../../../docs/design/plugins/Prompt.md#61-the-plugin-bridge".to_owned()
            )
        );
    }

    #[test]
    fn a_reference_definition_is_read_like_a_link() {
        let text = "/// [PromptSources §3]: ../../../docs/design/plugins/PromptSources.md\n"
            .to_owned()
            + "// [Quoted]: <../../../docs/design/plugins/Prompt.md> \"title\"\n"
            + "// see [Prompt §6.1]: ../../../docs/design/plugins/Prompt.md\n";
        let mut targets = Vec::new();
        for (_, comment) in comments(&text) {
            targets.extend(link_targets(&comment).into_iter().map(str::to_owned));
        }
        assert_eq!(
            targets,
            vec![
                "../../../docs/design/plugins/PromptSources.md",
                "../../../docs/design/plugins/Prompt.md",
            ]
        );
    }

    #[test]
    fn slugs_follow_github() {
        assert_eq!(
            slug("7.1.1 The parent block, `branch:parent`"),
            "711-the-parent-block-branchparent"
        );
        assert_eq!(
            slug("4.10 宿主端口：`ExecutionSettingsHost` 等五个"),
            "410-宿主端口executionsettingshost-等五个"
        );
        assert_eq!(slug("1. 规则"), "1-规则");
    }

    #[test]
    fn a_repeated_heading_is_suffixed_and_a_fenced_line_is_not_a_heading() {
        let text = "# One\n## One\n```\n# not a heading\n```\n";
        assert_eq!(
            headings(text),
            BTreeSet::from(["one".to_owned(), "one-1".to_owned()])
        );
    }
}
