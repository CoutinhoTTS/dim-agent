//! `check` fails when production source names a line with a bare `"main"`
//! string literal. The root's id is `ROOT_BRANCH_ID`; `DEFAULT_BRANCH_ID` is the
//! root, standing in for the default branch at a site that does not read the
//! Session's default branch. A use of the literal that is not about lines at
//! all — SQLite's `main` schema, an HTML tag, a layout variant, a git branch
//! name — is named in `UNRELATED` with its reason.
//!
//! The two ids are defined once, each in its own language's file; a definition
//! anywhere else is refused, so no private duplicate can hold the literal.
//!
//! Test files and items gated by a test-only `cfg` keep the literal: it is the
//! root's stored id, and the rule does not read them.
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The two ids' names.
const IDS: [&str; 2] = ["ROOT_BRANCH_ID", "DEFAULT_BRANCH_ID"];

/// Each language's file for the ids' one definition, and the spelling that file
/// uses.
const HOMES: [(&str, &str); 2] = [
    ("crates/dim-agent-types/src/events.rs", "pub const "),
    ("web/src/rpc/protocol.ts", "export const "),
];

/// Production uses of `"main"` that are not about lines, each with the file's
/// path from the repository root, text that must be on the literal's own line,
/// and why the use is unrelated.
const UNRELATED: [(&str, &str, &str); 9] = [
    (
        "crates/dim-app-server/src/storage/sqlite.rs",
        "is_readonly(\"main\")",
        "SQLite's main schema",
    ),
    (
        "crates/dim-app-server/src/provider/persistence.rs",
        "blob_open(\"main\"",
        "SQLite's main schema",
    ),
    (
        "crates/dim-app-server/src/web/content.rs",
        "extract_tag_inner(&cleaned, \"main\")",
        "an HTML tag",
    ),
    (
        "crates/dim-app-server/src/web/content.rs",
        "| \"main\" | \"blockquote\",",
        "an HTML tag",
    ),
    ("web/src/app/AppRoot.vue", "id=\"main\"", "an HTML tag"),
    ("web/src/app/AppShell.vue", "id=\"main\"", "an HTML tag"),
    (
        "web/src/features/chat/ComposerPanel.vue",
        "variant",
        "a layout variant",
    ),
    (
        "web/src/features/chat/TimelineView.vue",
        "variant",
        "a layout variant",
    ),
    ("web/src/i18n", "\"lines.main\"", "the root line's label"),
];

/// Checks every production source under `root`. Each failure names the file,
/// the line and where the id belongs.
pub fn check(root: &Path) -> Result<()> {
    let mut errors = Vec::new();
    let mut allowed = 0;
    for path in sources(root) {
        let text = fs::read_to_string(&path)?;
        let file = relative(root, &path);
        for offset in main_literals(&text) {
            let line = text[..offset].matches('\n').count() + 1;
            let source = text.lines().nth(line - 1).unwrap_or_default();
            if names_the_id(&file, source) {
                continue;
            }
            if defines_an_id(source) {
                errors.push(format!(
                    "{file}:{line}: a branch id is defined once, in {} or {}; \
                     name the definition's home in `HOMES` instead of copying it",
                    HOMES[0].0, HOMES[1].0
                ));
                continue;
            }
            if unrelated(&file, source) {
                allowed += 1;
                continue;
            }
            errors.push(format!(
                "{file}:{line}: `\"main\"` names neither the root nor the default branch; \
                 write `ROOT_BRANCH_ID` or `DEFAULT_BRANCH_ID`, or name the use in `UNRELATED`"
            ));
        }
    }
    if errors.is_empty() {
        println!("literal main passed ({allowed} unrelated uses)");
        Ok(())
    } else {
        Err(errors.join("\n").into())
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Whether `line` of `file` is one of the ids' definitions at its own home, in
/// the spelling that home uses: `pub const NAME: &str = "main"` in Rust and
/// `export const NAME = "main"` in TypeScript.
fn names_the_id(file: &str, line: &str) -> bool {
    HOMES
        .iter()
        .any(|(home, spelling)| file == *home && declares(line, spelling))
}

/// Whether `line` defines one of the ids, wherever it is written.
fn defines_an_id(line: &str) -> bool {
    ["pub const ", "export const ", "const "]
        .iter()
        .any(|spelling| declares(line, spelling))
}

/// Whether `line` declares one of the ids with `spelling`, which leads the line.
fn declares(line: &str, spelling: &str) -> bool {
    let Some(declared) = line.trim_start().strip_prefix(spelling) else {
        return false;
    };
    IDS.iter().any(|name| {
        declared.starts_with(name)
            && !declared[name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    })
}

/// Whether an allowlist entry covers the literal on `line` of `file`.
fn unrelated(file: &str, line: &str) -> bool {
    UNRELATED
        .iter()
        .any(|(path, text, _)| file.starts_with(path) && line.contains(text))
}

/// Every production source the rule reads: each crate's `src`, and the Web
/// workspace's `src`.
fn sources(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(root.join("crates")) {
        for entry in entries.flatten() {
            let directory = entry.path().join("src");
            if directory.is_dir() {
                collect(&directory, &mut files, Kind::Rust);
            }
        }
    }
    collect(&root.join("web/src"), &mut files, Kind::Web);
    files.sort();
    files
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rust,
    Web,
}

/// Collects the production files under `directory`, outside a `tests`
/// directory: a test module's sources are the root's stored id and are not
/// the rule's business.
fn collect(directory: &Path, files: &mut Vec<PathBuf>, kind: Kind) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if path.is_dir() {
            if name != "tests" {
                collect(&path, files, kind);
            }
        } else if is_production(&name, kind) {
            files.push(path);
        }
    }
}

/// Whether `name` is a production file the rule reads of its kind: a Rust file
/// that is not a test module, a Web source that is not a test.
fn is_production(name: &str, kind: Kind) -> bool {
    let is_test = name.contains(".test.") || name == "tests.rs";
    !is_test
        && match kind {
            Kind::Rust => name.ends_with(".rs"),
            Kind::Web => {
                name.ends_with(".ts")
                    || name.ends_with(".vue")
                    || name.ends_with(".mjs")
                    || name.ends_with(".js")
            }
        }
}

/// The byte offsets of the opening quote of every string literal in `text`
/// whose content is exactly `main`, outside comments and outside items gated by
/// a test-only `cfg`.
fn main_literals(text: &str) -> Vec<usize> {
    let skipped = test_only_items(text);
    literals(text)
        .into_iter()
        .filter(|(offset, content)| {
            *content == "main" && !skipped.iter().any(|range| range.contains(offset))
        })
        .map(|(offset, _)| offset)
        .collect()
}

/// Every string literal in `text` as its opening quote's offset and its
/// content. Comments and character literals are stepped over, so the quotes in
/// either are not literals.
fn literals(text: &str) -> Vec<(usize, &str)> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(character) = text[at..].chars().next() {
        if let Some(length) = comment(text, at) {
            at = (at + length).min(text.len());
        } else if let Some((start, end, length)) = string_literal(text, at) {
            found.push((at, &text[start..end]));
            at = (at + length).min(text.len());
        } else if let Some(length) = character_literal(text, at) {
            at = (at + length).min(text.len());
        } else {
            at += character.len_utf8();
        }
    }
    found
}

/// The length of the comment that starts at `at`, `None` when one does not. A
/// block comment nests as Rust's does.
fn comment(text: &str, at: usize) -> Option<usize> {
    let rest = &text[at..];
    if let Some(body) = rest.strip_prefix("//") {
        return Some(2 + body.find('\n').unwrap_or(body.len()));
    }
    let body = rest.strip_prefix("/*")?;
    let mut depth = 1;
    let mut index = 0;
    while index < body.len() {
        let rest = &body[index..];
        if rest.starts_with("/*") {
            depth += 1;
            index += 2;
        } else if rest.starts_with("*/") {
            depth -= 1;
            if depth == 0 {
                return Some(2 + index + 2);
            }
            index += 2;
        } else {
            index += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    Some(2 + body.len())
}

/// The string literal that starts at `at`, as its content's bounds and its
/// whole length, `None` when the first character starts none. The byte, C and
/// raw prefixes are read, and so is a raw identifier's `r#`, which is code.
fn string_literal(text: &str, at: usize) -> Option<(usize, usize, usize)> {
    let rest = &text[at..];
    if let Some(body) = rest.strip_prefix('"') {
        let content = quoted(body);
        return Some((at + 1, at + 1 + content, 1 + content + 1));
    }
    for prefix in ["br", "rb", "r", "b", "c"] {
        let Some(body) = rest.strip_prefix(prefix) else {
            continue;
        };
        let hashes = body.len() - body.trim_start_matches('#').len();
        let Some(body) = body[hashes..].strip_prefix('"') else {
            continue;
        };
        let terminator = format!("\"{}", "#".repeat(hashes));
        let end = body.find(&terminator).unwrap_or(body.len());
        let content = at + prefix.len() + hashes + 1;
        return Some((
            content,
            content + end,
            prefix.len()
                + hashes
                + 1
                + end
                + if end < body.len() {
                    terminator.len()
                } else {
                    0
                },
        ));
    }
    None
}

/// The bytes of `body` before its closing quote, escaped characters skipped:
/// the whole of `body` when it never closes.
fn quoted(body: &str) -> usize {
    let mut characters = body.char_indices();
    while let Some((offset, character)) = characters.next() {
        if character == '\\' {
            characters.next();
        } else if character == '"' {
            return offset;
        }
    }
    body.len()
}

/// The length of the character literal that starts at `at`, `None` for a
/// lifetime such as `'a`, which has no closing quote of its own.
fn character_literal(text: &str, at: usize) -> Option<usize> {
    let body = text[at..].strip_prefix('\'')?;
    let mut characters = body.char_indices();
    let (_, first) = characters.next()?;
    if first == '\\' {
        return quoted_with(body, '\'');
    }
    let end = first.len_utf8();
    body[end..].starts_with('\'').then_some(end + 2)
}

/// The bytes from the start of `body` through its closing `quote`, escaped
/// characters skipped, as the length of the literal including its opening
/// quote.
fn quoted_with(body: &str, quote: char) -> Option<usize> {
    let mut characters = body.char_indices();
    while let Some((offset, character)) = characters.next() {
        if character == '\\' {
            characters.next();
        } else if character == quote {
            return Some(1 + offset + 1);
        }
    }
    Some(1 + body.len())
}

/// The byte ranges of the items whose `cfg` holds only when `test` does: the
/// attribute and the item it annotates. A predicate that also holds without
/// `test` — `any(test, ...)`, `not(test)` — is production, so its item is not
/// skipped.
fn test_only_items(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut at = 0;
    while let Some(found) = text[at..].find("#[cfg(") {
        let open = at + found;
        at = open + 1;
        let Some((predicate, after_group)) = group(text, open + 5) else {
            continue;
        };
        if !test_only(predicate) {
            continue;
        }
        if let Some((start, end)) = item_range(text, after_group) {
            ranges.push(start..end);
        }
    }
    ranges
}

/// The predicate inside the parentheses that open at `open`, as its text and
/// the offset just past the closing parenthesis.
fn group(text: &str, open: usize) -> Option<(&str, usize)> {
    let body = text.get(open..)?.strip_prefix('(')?;
    let start = open + 1;
    let mut depth = 1;
    for (index, byte) in body.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&text[start..start + index], start + index + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether a `cfg` predicate holds only when `test` does.
fn test_only(predicate: &str) -> bool {
    let predicate = predicate.trim();
    if predicate == "test" {
        return true;
    }
    let Some(inner) = predicate
        .strip_prefix("all(")
        .and_then(|inner| inner.strip_suffix(')'))
    else {
        return false;
    };
    inner.split(',').any(test_only)
}

/// The range of the item annotated by an attribute whose predicate closes just
/// before `after_group`: its opening `#[` through the item's closing brace, or
/// through its `;` when the item is a declaration.
fn item_range(text: &str, after_group: usize) -> Option<(usize, usize)> {
    let open = text[..after_group].rfind("#[")?;
    let mut at = after_group;
    while at < text.len() && text.as_bytes()[at] != b']' {
        at += 1;
    }
    let mut depth = 0usize;
    for (offset, character) in text[at..].char_indices() {
        match character {
            '{' if depth == 0 => return Some((open, closing_brace(text, at + offset) + 1)),
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => return Some((open, at + offset + 1)),
            _ => {}
        }
    }
    None
}

/// The offset of the brace that closes the block opening at `open`.
fn closing_brace(text: &str, open: usize) -> usize {
    let mut depth = 0usize;
    for (offset, character) in text[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return open + offset;
                }
            }
            _ => {}
        }
    }
    text.len().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dim-xtask-literal-main-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        root
    }

    fn refusal(root: &Path) -> String {
        let error = check(root).unwrap_err().to_string();
        std::fs::remove_dir_all(root).unwrap();
        error
    }

    #[test]
    fn a_bare_main_in_production_is_refused_with_its_line() {
        let root = tree(
            "bare",
            &[(
                "crates/example/src/lib.rs",
                "fn branch() -> &'static str {\n    \"main\"\n}\n",
            )],
        );
        let error = refusal(&root);
        assert!(
            error.contains("crates/example/src/lib.rs:2: `\"main\"` names neither"),
            "{error}"
        );
    }

    #[test]
    fn the_constants_and_an_allowlisted_use_pass() {
        let root = tree(
            "allowed",
            &[
                (
                    "crates/dim-agent-types/src/events.rs",
                    "pub const ROOT_BRANCH_ID: &str = \"main\";\n\
                     pub const DEFAULT_BRANCH_ID: &str = ROOT_BRANCH_ID;\n",
                ),
                (
                    "web/src/rpc/protocol.ts",
                    "export const ROOT_BRANCH_ID = \"main\"\n\
                     export const DEFAULT_BRANCH_ID = ROOT_BRANCH_ID\n",
                ),
                (
                    "crates/dim-app-server/src/storage/sqlite.rs",
                    "if let Ok(false) = connection.is_readonly(\"main\") {}\n",
                ),
            ],
        );
        check(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_definition_outside_its_home_is_refused_with_its_line() {
        let root = tree(
            "duplicate",
            &[(
                "crates/example/src/lib.rs",
                "fn root() -> &'static str { ROOT_BRANCH_ID }\n\
                 const ROOT_BRANCH_ID: &str = \"main\";\n",
            )],
        );
        let error = refusal(&root);
        assert!(
            error.contains("crates/example/src/lib.rs:2: a branch id is defined once"),
            "{error}"
        );
    }

    #[test]
    fn a_test_module_and_a_test_file_are_skipped() {
        let root = tree(
            "test",
            &[
                (
                    "crates/example/src/lib.rs",
                    "fn root() -> &'static str { ROOT_BRANCH_ID }\n\
                     #[cfg(test)]\n\
                     mod tests {\n    const ID: &str = \"main\";\n}\n\
                     #[cfg(all(test, unix))]\n\
                     mod unix_tests {\n    const ID: &str = \"main\";\n}\n",
                ),
                (
                    "crates/example/src/agent/tests.rs",
                    "fn fixture() -> String { \"main\".into() }\n",
                ),
            ],
        );
        check(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_allowlisted_use_of_another_kind_passes_and_a_new_one_fails() {
        let root = tree(
            "kinds",
            &[
                (
                    "crates/dim-app-server/src/web/content.rs",
                    "let root = extract_tag_inner(&cleaned, \"main\");\n\
                     match (false, name) {\n    \"p\" | \"main\" | \"blockquote\",\n} => 1,\n",
                ),
                (
                    "crates/example/src/lib.rs",
                    "fn other() -> &'static str { \"main\" }\n",
                ),
            ],
        );
        let error = refusal(&root);
        assert!(error.contains("crates/example/src/lib.rs:1"), "{error}");
        assert!(!error.contains("web/content.rs"), "{error}");
    }

    #[test]
    fn a_cfg_that_also_holds_without_test_is_production() {
        let root = tree(
            "cfg",
            &[(
                "crates/example/src/lib.rs",
                "#[cfg(any(test, feature = \"x\"))]\n\
                 fn a() -> &'static str { \"main\" }\n",
            )],
        );
        let error = refusal(&root);
        assert!(error.contains("crates/example/src/lib.rs:2"), "{error}");
    }

    #[test]
    fn a_main_that_only_looks_like_one_is_not_a_literal() {
        let root = tree(
            "lexical",
            &[(
                "crates/example/src/lib.rs",
                "/// `session.service::<Timeline>(\"main\")` is a doc example.\n\
                 // a comment: \"main\"\n\
                 fn sample() -> &'static str { r#\"<dim-branch-line parent=\"main\">\"# }\n",
            )],
        );
        check(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
