//! `check` fails when a locale table does not hold the Simplified Chinese
//! table's keys, or when a translation call (`t`, `tf`, `t_in`, `tf_in`) or a
//! `ListPanel` text passes a key the Simplified Chinese table lacks. A key is
//! the English text itself (`crates/dim-cli/src/ui/i18n/mod.rs`), so a table is
//! a `lookup` function whose match arms are string literals, and a call site is
//! a key argument that reduces to string literals: one literal, or the branches
//! of an `if`, `match` or block. `ListPanel` keeps its title, loading text,
//! empty text and no-match text English until it draws, so those arguments are
//! keys too.
//!
//! Both sides are parsed, not matched by hand: a table arm whose pattern is
//! neither a string nor the catch-all is a failure, so a new shape cannot pass
//! by being skipped. A key argument that reduces to no literal is reported
//! rather than failed: the ones left name constants, which the check does not
//! follow.
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use syn::{
    Block, Expr, ExprLit, Item, Lit, Pat, Stmt,
    spanned::Spanned,
    visit::{self, Visit},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The locale tables, one directory per locale, one file per area.
const TABLES: &str = "crates/dim-cli/src/ui/i18n";
/// The sources whose translation calls are checked.
const SOURCE: &str = "crates/dim-cli/src";
/// The table every locale must hold the keys of.
const BASE: &str = "zh_cn";
/// The `ListPanel::new` arguments that hold text the panel translates when it
/// draws: the title, the loading text, the empty text and the no-match text.
const LIST_PANEL_TEXTS: &[usize] = &[0, 2, 3, 4];

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
    for (key, site) in &calls.keys {
        if !base.contains(key) {
            errors.push(format!("{site}: {key:?} is not in {BASE}"));
        }
    }
    if errors.is_empty() {
        println!(
            "i18n keys passed ({} tables, {} literal keys, {} key arguments without a literal)",
            tables.len(),
            calls.keys.len(),
            calls.unreadable.len()
        );
        for site in &calls.unreadable {
            println!("  {site}: no literal key");
        }
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

/// Every key the sources pass to a translation call or to a `ListPanel` text,
/// and every key argument that reduces to no literal.
fn calls(root: &Path) -> Result<CallKeys> {
    let tables = root.join(TABLES);
    let mut found = CallKeys::default();
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

/// What the calls in one source pass: each key with the site that passes it,
/// and the site of each key argument the check cannot read.
#[derive(Default)]
struct CallKeys {
    keys: Vec<(String, String)>,
    unreadable: Vec<String>,
}

impl CallKeys {
    fn extend(&mut self, other: Self) {
        for key in other.keys {
            if !self.keys.contains(&key) {
                self.keys.push(key);
            }
        }
        for site in other.unreadable {
            if !self.unreadable.contains(&site) {
                self.unreadable.push(site);
            }
        }
    }
}

/// Every literal key in `text` (`site` names it in the result), with the line
/// it is passed on.
fn call_keys(text: &str, site: &str) -> Result<CallKeys> {
    let file: syn::File = syn::parse_file(text)?;
    let mut visitor = Calls {
        site: site.to_owned(),
        found: CallKeys::default(),
    };
    visitor.visit_file(&file);
    Ok(visitor.found)
}

/// The argument positions of a call that hold a translation key, when its name
/// is one the check reads.
fn key_positions(function: &Expr) -> Option<&'static [usize]> {
    let Expr::Path(function) = function else {
        return None;
    };
    let mut segments = function.path.segments.iter().rev();
    match segments.next()?.ident.to_string().as_str() {
        "t" | "tf" => Some(&[0]),
        "t_in" | "tf_in" => Some(&[1]),
        "new" if segments.next().is_some_and(|module| module.ident == "ListPanel") => {
            Some(LIST_PANEL_TEXTS)
        }
        _ => None,
    }
}

struct Calls {
    site: String,
    found: CallKeys,
}

impl Calls {
    /// Adds every literal `expr` reduces to, looking through the branches of an
    /// `if`, `match` or block, a parenthesized expression and a nested
    /// translation call.
    fn literals(&mut self, expr: &Expr) {
        match expr {
            Expr::Lit(ExprLit {
                lit: Lit::Str(key), ..
            }) => self.key(key),
            Expr::If(node) => {
                self.block(&node.then_branch);
                if let Some((_, otherwise)) = &node.else_branch {
                    self.literals(otherwise);
                }
            }
            Expr::Match(node) => {
                for arm in &node.arms {
                    self.literals(&arm.body);
                }
            }
            Expr::Block(node) => self.block(&node.block),
            Expr::Paren(node) => self.literals(&node.expr),
            Expr::Group(node) => self.literals(&node.expr),
            Expr::Reference(node) => self.literals(&node.expr),
            Expr::Return(node) => {
                if let Some(value) = &node.expr {
                    self.literals(value);
                }
            }
            Expr::Call(node) => match key_positions(&node.func) {
                Some(positions) => {
                    for position in positions {
                        if let Some(argument) = node.args.get(*position) {
                            self.literals(argument);
                        }
                    }
                }
                None => self.unreadable(expr.span()),
            },
            _ => self.unreadable(expr.span()),
        }
    }

    /// Adds the literal a block evaluates to.
    fn block(&mut self, block: &Block) {
        match block.stmts.last() {
            Some(Stmt::Expr(value, None)) => self.literals(value),
            _ => self.unreadable(block.brace_token.span.open()),
        }
    }

    /// Adds `key`, unless it is empty: a panel draws nothing for one, so no
    /// table holds it.
    fn key(&mut self, key: &syn::LitStr) {
        let text = key.value();
        if text.is_empty() {
            return;
        }
        let found = (text, format!("{}:{}", self.site, key.span().start().line));
        if !self.found.keys.contains(&found) {
            self.found.keys.push(found);
        }
    }

    fn unreadable(&mut self, span: proc_macro2::Span) {
        let site = format!("{}:{}", self.site, span.start().line);
        if !self.found.unreadable.contains(&site) {
            self.found.unreadable.push(site);
        }
    }
}

impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let Some(positions) = key_positions(&node.func) {
            for position in positions {
                if let Some(argument) = node.args.get(*position) {
                    self.literals(argument);
                }
            }
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "set_empty_text"
            && let Some(argument) = node.args.first()
        {
            self.literals(argument);
        }
        visit::visit_expr_method_call(self, node);
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
            keys.keys,
            vec![
                ("Queue".to_owned(), "src.rs:2".to_owned()),
                ("Held {id}".to_owned(), "src.rs:3".to_owned()),
                ("Nested".to_owned(), "src.rs:5".to_owned()),
                ("Late".to_owned(), "src.rs:7".to_owned()),
            ]
        );
    }

    /// A key argument collects the literal of every branch it can take, and a
    /// branch that is not a literal is reported.
    #[test]
    fn the_literals_of_every_branch_are_keys() {
        let text = "fn draw(preset: Preset, confirm: bool, name: &str) {\n    let a = t(match preset {\n        Preset::ReadOnly => \"Read only\",\n        Preset::FullAccess => {\n            \"Full access\"\n        }\n    });\n    let b = tf(if confirm { \"Held {id}\" } else { \"Released {id}\" }, &[\"id\"]);\n    let c = t((\"Parenthesized\"));\n    let d = t(name);\n}\n";
        let keys = call_keys(text, "src.rs").unwrap();
        assert_eq!(
            keys.keys,
            vec![
                ("Read only".to_owned(), "src.rs:3".to_owned()),
                ("Full access".to_owned(), "src.rs:5".to_owned()),
                ("Held {id}".to_owned(), "src.rs:8".to_owned()),
                ("Released {id}".to_owned(), "src.rs:8".to_owned()),
                ("Parenthesized".to_owned(), "src.rs:9".to_owned()),
            ]
        );
        assert_eq!(keys.unreadable, vec!["src.rs:10".to_owned()]);
    }

    /// The title, loading, empty and no-match text a `ListPanel` holds until it
    /// draws are keys; its hint and its other constructor arguments are not.
    #[test]
    fn the_texts_a_list_panel_translates_when_it_draws_are_keys() {
        let text = "fn panel(hint: &str) {\n    let mut list = ListPanel::new(\"Title\", hint, \"Loading...\", \"Empty.\", \"No match.\", false);\n    list.set_empty_text(\"Nothing here.\");\n}\n";
        let keys = call_keys(text, "src.rs").unwrap();
        assert_eq!(
            keys.keys,
            vec![
                ("Title".to_owned(), "src.rs:2".to_owned()),
                ("Loading...".to_owned(), "src.rs:2".to_owned()),
                ("Empty.".to_owned(), "src.rs:2".to_owned()),
                ("No match.".to_owned(), "src.rs:2".to_owned()),
                ("Nothing here.".to_owned(), "src.rs:3".to_owned()),
            ]
        );
    }

    /// A root whose Simplified Chinese table holds `table` and whose one source
    /// file passes `source`, so `check` reads both.
    fn root(name: &str, table: &str, source: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dim-xtask-i18n-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let areas = root.join(TABLES).join(BASE);
        std::fs::create_dir_all(&areas).unwrap();
        std::fs::write(areas.join("panel.rs"), table).unwrap();
        let sources = root.join(SOURCE).join("ui");
        std::fs::create_dir_all(&sources).unwrap();
        std::fs::write(sources.join("panel.rs"), source).unwrap();
        root
    }

    fn refusal(root: &Path) -> String {
        let error = check(root)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        std::fs::remove_dir_all(root).unwrap();
        error
    }

    const TABLE: &str = r#"pub fn lookup(english: &str) -> Option<&'static str> {
    Some(match english {
        "Kept" => "保留",
        _ => return None,
    })
}"#;

    #[test]
    fn a_key_missing_from_a_match_branch_is_refused_by_name() {
        let root = root(
            "match-branch",
            TABLE,
            "fn draw(preset: Preset) {\n    let label = t(match preset {\n        Preset::A => \"Kept\",\n        Preset::B => \"Missing from a match branch\",\n    });\n}\n",
        );
        let error = refusal(&root);
        assert!(
            error.contains(
                "crates/dim-cli/src/ui/panel.rs:4: \"Missing from a match branch\" is not in zh_cn"
            ),
            "{error}"
        );
    }

    #[test]
    fn a_key_missing_from_an_if_branch_is_refused_by_name() {
        let root = root(
            "if-branch",
            TABLE,
            "fn draw(confirm: bool) {\n    let label = t(if confirm {\n        \"Kept\"\n    } else {\n        \"Missing from an if branch\"\n    });\n}\n",
        );
        let error = refusal(&root);
        assert!(
            error.contains(
                "crates/dim-cli/src/ui/panel.rs:5: \"Missing from an if branch\" is not in zh_cn"
            ),
            "{error}"
        );
    }

    #[test]
    fn a_panel_text_missing_from_the_base_table_is_refused_by_name() {
        let root = root(
            "panel-text",
            TABLE,
            "fn panel(hint: &str) {\n    let list = ListPanel::new(\"Missing panel title\", hint, \"\", \"\", \"\", false);\n    drop(list);\n}\n",
        );
        let error = refusal(&root);
        assert!(
            error.contains("crates/dim-cli/src/ui/panel.rs:2: \"Missing panel title\" is not in zh_cn"),
            "{error}"
        );
    }
}
