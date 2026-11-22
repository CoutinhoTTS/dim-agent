//! The exact version the shipped `plugin-authoring` Skill gives the SDK.
//!
//! The Skill teaches an author to install the SDK into the plugin's own package,
//! and rutis refuses a peer that speaks another session protocol at the
//! handshake (ADR 025), so the version the Skill writes for `@arcships/rutis` is
//! the one `runtime/package.json` declares.

use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SKILL: &str = "runtime/plugins/plugin-authoring/skills/plugin-authoring/SKILL.md";
const SDK: &str = "@arcships/rutis";

/// Every version `text` gives the SDK, such as `0.23.0`.
fn versions(text: &str) -> Vec<&str> {
    let mut named = Vec::new();
    for (at, _) in text.match_indices(SDK) {
        let after = &text[at + SDK.len()..];
        if after.starts_with(|c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '/')) {
            continue;
        }
        let rest = after.trim_start_matches(|c: char| {
            matches!(c, '"' | '\'' | '`' | ':' | '=' | '@' | ' ' | '\t')
        });
        if let Some(version) = version_at(rest)
            && !named.contains(&version)
        {
            named.push(version);
        }
    }
    named
}

/// The three-part version at the start of `text`, such as `0.23.0`.
fn version_at(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut end = 0;
    for part in 0..3 {
        let start = end;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end == start {
            return None;
        }
        if part < 2 {
            if bytes.get(end) != Some(&b'.') {
                return None;
            }
            end += 1;
        }
    }
    Some(&text[..end])
}

/// Whether `version` pins all three components, such as `0.23.0`.
fn is_exact(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

pub fn check(root: &Path) -> Result<()> {
    let runtime: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("runtime/package.json"))?)?;
    let expected = runtime["devDependencies"][SDK]
        .as_str()
        .ok_or("runtime/package.json declares no @arcships/rutis version")?;
    if !is_exact(expected) {
        return Err(format!(
            "runtime/package.json must pin {SDK} to an exact X.Y.Z version, not {expected}"
        )
        .into());
    }
    let skill = std::fs::read_to_string(root.join(SKILL))?;
    let named = versions(&skill);
    if named.is_empty() {
        return Err(
            format!("{SKILL}: names no version; runtime/package.json declares {expected}").into(),
        );
    }
    let mut stale: Vec<&str> = Vec::new();
    for version in named {
        if version != expected && !stale.contains(&version) {
            stale.push(version);
        }
    }
    if !stale.is_empty() {
        return Err(format!(
            "{SKILL}: names {}; runtime/package.json declares {expected}",
            stale.join(", ")
        )
        .into());
    }
    println!("plugin skill versions passed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree whose runtime manifest declares 0.23.0 and whose Skill is `skill`.
    fn tree(name: &str, skill: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dim-xtask-plugin-skill-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join(SKILL).parent().unwrap()).unwrap();
        std::fs::write(
            root.join("runtime/package.json"),
            r#"{"devDependencies":{"@arcships/rutis":"0.23.0"}}"#,
        )
        .unwrap();
        std::fs::write(root.join(SKILL), skill).unwrap();
        root
    }

    #[test]
    fn a_version_the_runtime_does_not_declare_is_named() {
        let root = tree(
            "stale",
            concat!(
                "the SDK version must be exactly the one Dim runs, 0.17.1:\n\n",
                "```json\n{ \"dependencies\": { \"@arcships/rutis\": \"0.17.1\" } }\n```\n",
            ),
        );
        let error = check(&root).unwrap_err().to_string();
        std::fs::remove_dir_all(root).unwrap();
        assert!(
            error.contains("names 0.17.1; runtime/package.json declares 0.23.0"),
            "{error}"
        );
    }

    #[test]
    fn a_skill_that_names_no_version_is_refused() {
        let root = tree(
            "no-version",
            "```json\n{ \"dependencies\": { \"@arcships/rutis\": \"latest\" } }\n```\n",
        );
        let error = check(&root).unwrap_err().to_string();
        std::fs::remove_dir_all(root).unwrap();
        assert!(error.contains("names no version"), "{error}");
    }

    #[test]
    fn the_runtime_version_passes() {
        let root = tree(
            "current",
            "```json\n{ \"dependencies\": { \"@arcships/rutis\": \"0.23.0\" } }\n```\n",
        );
        check(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_version_that_ends_a_sentence_is_named() {
        let root = tree(
            "sentence",
            "the SDK version must be exactly the one Dim runs: @arcships/rutis 0.17.1.\n",
        );
        let error = check(&root).unwrap_err().to_string();
        std::fs::remove_dir_all(root).unwrap();
        assert!(error.contains("names 0.17.1;"), "{error}");
    }

    #[test]
    fn versions_given_for_other_packages_are_ignored() {
        let root = tree(
            "other-packages",
            concat!(
                "{ \"name\": \"echo-plugin\", \"version\": \"1.0.0\",\n",
                "  \"engines\": { \"node\": \">=20.19.0\" },\n",
                "  \"dependencies\": {\n",
                "    \"@arcships/rutis\": \"0.23.0\",\n",
                "    \"@arcships/rutis-host\": \"0.22.0\",\n",
                "    \"@modelcontextprotocol/sdk\": \"1.31.0\",\n",
                "    \"zod\": \"4.6.7\"\n",
                "  } }\n",
            ),
        );
        check(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_runtime_manifest_that_does_not_pin_an_exact_version_is_refused() {
        for declared in ["^0.23.0", "~0.23.0", ">=0.23.0"] {
            let root = tree(
                "range",
                "```json\n{ \"dependencies\": { \"@arcships/rutis\": \"0.23.0\" } }\n```\n",
            );
            std::fs::write(
                root.join("runtime/package.json"),
                format!(r#"{{"devDependencies":{{"@arcships/rutis":"{declared}"}}}}"#),
            )
            .unwrap();
            let error = check(&root).unwrap_err().to_string();
            std::fs::remove_dir_all(root).unwrap();
            assert!(
                error.contains(&format!(
                    "must pin @arcships/rutis to an exact X.Y.Z version, not {declared}"
                )),
                "{error}"
            );
        }
    }
}
