//! `check` fails when a locale table does not hold the Simplified Chinese
//! table's keys, or when a translation call (`t`, `tf`, `t_in`, `tf_in`)
//! passes a literal key that the Simplified Chinese table lacks. A key is the
//! English text itself (`crates/dim-cli/src/ui/i18n/mod.rs`), so a table is a
//! `lookup` function whose match arms are string literals, and a call site is a
//! literal key argument.
//!
//! Both sides are parsed, not matched by hand: a table arm whose pattern is
//! neither a string nor the catch-all is a failure, so a new shape cannot pass
//! by being skipped.
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use syn::{
    Expr, ExprLit, Item, Lit, Pat, Stmt,
    visit::{self, Visit},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The locale tables, one directory per locale, one file per area.
const TABLES: &str = "crates/dim-cli/src/ui/i18n";
/// The sources whose translation calls are checked.
const SOURCE: &str = "crates/dim-cli/src";
/// The table every locale must hold the keys of.
const BASE: &str = "zh_cn";

/// Checks every locale table and every literal call key. Each failure names
/// the table or the call site and the key.
pub fn check(root: &Path) -> Result<()> {
    let tables = tables(root)?;
    let base = tables
        .get(BASE)
        .ok_or_else(|| format!("{TABLES}/{BASE}: no table"))?;
    let mut errors = Vec::new();
    for (locale, keys) in &tables {
        if locale == BASE {
            continue;
        }
        for key in base.difference(keys) {
            errors.push(format!("{TABLES}/{locale}: lacks {key:?}"));
        }
        for key in keys.difference(base) {
            errors.push(format!("{TABLES}/{locale}: {key:?} is not in {BASE}"));
        }
    }
    let calls = calls(root)?;
    for (key, site) in &calls {
        if !base.contains(key) {
            errors.push(format!("{site}: {key:?} is not in {BASE}"));
        }
    }
    if errors.is_empty() {
        println!(
            "i18n keys passed ({} tables, {} literal calls)",
            tables.len(),
            calls.len()
        );
        Ok(())
    } else {
        Err(errors.join("\n").into())
    }
}

/// Every locale's keys, read from its `lookup` arms.
fn tables(root: &Path) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut tables = BTreeMap::new();
    for entry in fs::read_dir(root.join(TABLES))? {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let mut keys = BTreeSet::new();
        for area in sources(&path) {
            if path_name(&area) == "mod.rs" {
                continue;
            }
            let name = area.display().to_string();
            keys.extend(table_keys(&fs::read_to_string(&area)?, &name)?);
        }
        tables.insert(path_name(&path), keys);
    }
    Ok(tables)
}

/// The keys of one area table (`text`, named `name` in failures): the string
/// literals its `lookup` match arms test, with the catch-all arm skipped.
fn table_keys(text: &str, name: &str) -> Result<BTreeSet<String>> {
    let file: syn::File = syn::parse_file(text)?;
    let function = file
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "lookup" => Some(function),
            _ => None,
        })
        .ok_or_else(|| format!("{name}: no lookup function"))?;
    let Some(Stmt::Expr(Expr::Call(call), None)) = function.block.stmts.last() else {
        return Err(format!("{name}: lookup does not return Some(match ..)").into());
    };
    let Some(Expr::Match(expression)) = call.args.first() else {
        return Err(format!("{name}: lookup does not return Some(match ..)").into());
    };
    let mut keys = BTreeSet::new();
    for arm in &expression.arms {
        match &arm.pat {
            Pat::Lit(literal) => match &literal.lit {
                Lit::Str(key) => {
                    keys.insert(key.value());
                }
                _ => return Err(format!("{name}: an arm's key is not a string").into()),
            },
            Pat::Wild(_) => {}
            _ => {
                return Err(format!(
                    "{name}: an arm's pattern is neither a string nor the catch-all"
                )
                .into());
            }
        }
    }
    Ok(keys)
}

/// Every literal key passed to a translation call under the sources, as the
/// key and the site that passes it.
fn calls(root: &Path) -> Result<Vec<(String, String)>> {
    let tables = root.join(TABLES);
    let mut found = Vec::new();
    for path in sources(&root.join(SOURCE)) {
        if path.starts_with(&tables) {
            continue;
        }
        let site = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        found.extend(call_keys(&fs::read_to_string(&path)?, &site)?);
    }
    Ok(found)
}

/// Every literal translation key in `text` (`site` names it in the result),
/// with the line it is passed on.
fn call_keys(text: &str, site: &str) -> Result<Vec<(String, String)>> {
    let file: syn::File = syn::parse_file(text)?;
    let mut visitor = Calls {
        site: site.to_owned(),
        keys: Vec::new(),
    };
    visitor.visit_file(&file);
    Ok(visitor.keys)
}

/// The index of the argument a call names as its key, when its name is one of
/// the translation calls.
fn key_argument(name: &str) -> Option<usize> {
    match name {
        "t" | "tf" => Some(0),
        "t_in" | "tf_in" => Some(1),
        _ => None,
    }
}

struct Calls {
    site: String,
    keys: Vec<(String, String)>,
}

impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let Expr::Path(function) = &*node.func {
            let index = function
                .path
                .segments
                .last()
                .and_then(|segment| key_argument(&segment.ident.to_string()));
            if let Some(Expr::Lit(ExprLit {
                lit: Lit::Str(key), ..
            })) = index.and_then(|index| node.args.get(index))
            {
                self.keys.push((
                    key.value(),
                    format!("{}:{}", self.site, key.span().start().line),
                ));
            }
        }
        visit::visit_expr_call(self, node);
    }
}

/// The Rust sources under `directory`, recursively, sorted.
fn sources(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    sources_in(directory, &mut files);
    files.sort();
    files
}

fn sources_in(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("a source directory reads") {
        let path = entry.expect("a source entry reads").path();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Result<BTreeSet<String>> {
        table_keys(text, "table.rs")
    }

    #[test]
    fn a_table_reads_every_literal_arm_and_skips_the_catch_all() {
        let keys = table(
            r#"pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Queue" => "Warteschlange",
        "a \"quoted\" key" => {
            "Wert"
        }
        _ => return None,
    })
}"#,
        )
        .unwrap();
        assert_eq!(
            keys,
            BTreeSet::from(["Queue".to_owned(), "a \"quoted\" key".to_owned()])
        );
    }

    #[test]
    fn an_arm_that_is_not_a_string_is_a_failure() {
        let error = table(
            r#"pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        KEY => "Wert",
        _ => return None,
    })
}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("neither a string"), "{error}");
    }

    #[test]
    fn a_literal_call_key_is_found_and_a_variable_is_not() {
        let text = "fn draw(name: &str) {\n    let a = t(\"Queue\");\n    let b = tf(\"Held {id}\", &[\"id\"]);\n    let c = t(name);\n    let d = i18n::t(\"Nested\");\n    let e = other.t(\"Method\");\n    let f = t_in(locale, \"Late\");\n}\n";
        let keys = call_keys(text, "src.rs").unwrap();
        assert_eq!(
            keys,
            vec![
                ("Queue".to_owned(), "src.rs:2".to_owned()),
                ("Held {id}".to_owned(), "src.rs:3".to_owned()),
                ("Nested".to_owned(), "src.rs:5".to_owned()),
                ("Late".to_owned(), "src.rs:7".to_owned()),
            ]
        );
    }
}
