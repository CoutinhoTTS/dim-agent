//! Checks hand-written plugin dependency tables against the live local execution profile.

use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const INFRASTRUCTURE: [&str; 12] = [
    "rutis-loader",
    "session",
    "branch",
    "SessionScope",
    "BranchScope",
    // The instances' mounters (ADR 006 D3); the parts they mount have pages.
    "SessionParts",
    "SessionOptionalParts",
    "BranchParts",
    "BranchOptionalParts",
    "SessionBroadcast",
    "SessionBroadcastEnd",
    "PrepareModelHost",
];

pub fn check(root: &Path, graphs: &BTreeMap<&str, Value>) -> Result<()> {
    let dir = root.join("docs/architecture/rust-agent/plugins");
    let mut plugins = BTreeMap::new();
    for graph in graphs.values() {
        for plugin in graph["plugins"]
            .as_array()
            .ok_or("missing profile plugins")?
        {
            let name = plugin["name"].as_str().ok_or("missing plugin name")?;
            let mut injected = BTreeSet::new();
            for inject in plugin["injects"]
                .as_array()
                .ok_or("missing plugin injects")?
            {
                injected.insert(code(
                    inject["service"].as_str().ok_or("missing service key")?,
                ));
            }
            plugins.insert(name, injected);
        }
    }
    let mut documented = BTreeSet::new();
    let mut errors = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "md")
            || path.file_stem().is_some_and(|s| s == "README")
        {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let tables = dependency_tables(&text);
        if tables.is_empty() {
            errors.push(format!("{}: no dependency table", path.display()));
        }
        for (name, explained) in tables {
            documented.insert(name.to_owned());
            let Some(injected) = plugins.get(name) else {
                errors.push(format!(
                    "{}: plugin `{name}` is not in the local execution profile",
                    path.display()
                ));
                continue;
            };
            for missing in injected.difference(&explained) {
                errors.push(format!(
                    "{}: `{name}`: dependency table does not explain injected `{missing}`",
                    path.display()
                ));
            }
            for extra in explained.difference(injected) {
                errors.push(format!(
                    "{}: `{name}`: dependency table explains `{extra}`, which it does not inject",
                    path.display()
                ));
            }
        }
    }
    for name in plugins.keys() {
        if !INFRASTRUCTURE.contains(name) && !documented.contains(*name) {
            errors.push(format!(
                "business plugin `{name}` has no document in {}",
                dir.display()
            ));
        }
    }
    if documented.is_empty() {
        errors.push(format!("{}: no documented plugins", dir.display()));
    }
    if errors.is_empty() {
        println!("plugin documents passed ({} plugins)", documented.len());
        Ok(())
    } else {
        errors.sort();
        Err(errors.join("\n").into())
    }
}

fn dependency_tables(text: &str) -> Vec<(&str, BTreeSet<String>)> {
    let mut page = "";
    let mut section = "";
    let mut tables = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((depth, title)) = line.split_once(' ') else {
            continue;
        };
        let name = match (depth, title) {
            ("##", "Dependencies" | "依赖") => page,
            ("###", "Dependencies" | "依赖") => section,
            ("#", _) => {
                page = title;
                continue;
            }
            ("##", _) => {
                section = title;
                continue;
            }
            _ => continue,
        };
        let mut rows = BTreeSet::new();
        while let Some(line) = lines.next_if(|line| !line.starts_with('#')) {
            if let Some(cell) = line
                .strip_prefix('|')
                .and_then(|rest| rest.split('|').next())
                && let Some(key) = cell
                    .trim()
                    .strip_prefix('`')
                    .and_then(|key| key.strip_suffix('`'))
            {
                rows.insert(key.to_owned());
            }
        }
        tables.push((name, rows));
    }
    tables
}

/// `dim_agent_x::m::Name<dim_agent_y::T>#key@main` → `B:Name<T>#key`.
fn code(service: &str) -> String {
    let (path, scope) = match service.rsplit_once('@') {
        Some((path, "main")) => (path, "B"),
        Some((path, "session")) => (path, "S"),
        Some((path, _)) => (path, "?"),
        None => (service, "P"),
    };
    let mut short = String::new();
    let mut segment = String::new();
    let mut chars = path.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == ':' && chars.peek() == Some(&':') {
            chars.next();
            segment.clear();
            continue;
        }
        if ch.is_alphanumeric() || ch == '_' {
            segment.push(ch);
            continue;
        }
        short.push_str(&segment);
        segment.clear();
        short.push(ch);
    }
    short.push_str(&segment);
    format!("{scope}:{short}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documentation_check_requires_complete_profile_and_exact_dependencies() {
        let root = std::env::temp_dir().join(format!(
            "dim-plugin-doc-check-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let result = (|| -> Result<()> {
            let mut graphs = BTreeMap::from([(
                "main",
                serde_json::json!({
                    "plugins": [
                        {"id":1,"name":"branch","injects":[],"resolved":[]},
                        {"id":2,"name":"Example","injects":[{"service":"crate::Timeline@main"}],"resolved":[]}
                    ],
                    "bindings":[]
                }),
            )]);
            assert!(
                check(&root, &graphs).is_err(),
                "missing directory must not pass"
            );
            let dir = root.join("docs/architecture/rust-agent/plugins");
            std::fs::create_dir_all(&dir)?;
            assert!(
                check(&root, &graphs).is_err(),
                "empty document set must not pass"
            );
            let path = dir.join("Example.md");
            let page = "# Example\n\n## Dependencies\n\n| Service | Purpose |\n| --- | --- |\n| `B:Timeline` | Commit results |\n";
            std::fs::write(&path, page)?;
            check(&root, &graphs)?;
            let valid = std::fs::read_to_string(&path)?;
            std::fs::write(
                &path,
                valid.replace("| `B:Timeline` | Commit results |", ""),
            )?;
            assert!(
                check(&root, &graphs).is_err(),
                "a missing dependency explanation must fail"
            );
            std::fs::write(&path, &valid)?;
            graphs.get_mut("main").unwrap()["plugins"]
                .as_array_mut()
                .unwrap()
                .push(
                    serde_json::json!({"id":3,"name":"NewCapability","injects":[],"resolved":[]}),
                );
            assert!(
                check(&root, &graphs).is_err(),
                "new business plugins require documentation"
            );
            graphs.get_mut("main").unwrap()["plugins"]
                .as_array_mut()
                .unwrap()
                .pop();
            std::fs::write(dir.join("Removed.md"), page.replace("Example", "Removed"))?;
            assert!(
                check(&root, &graphs).is_err(),
                "removed plugins must not retain orphan documents"
            );
            std::fs::remove_file(dir.join("Removed.md"))?;
            check(&root, &graphs)
        })();
        std::fs::remove_dir_all(root).unwrap();
        result.unwrap();
    }
}
