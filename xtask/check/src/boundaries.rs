//! Crate and module dependency boundaries from Cargo metadata and Rust source.

use crate::modules;
use serde_json::{Value, json};
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn check_dependency(name: &str, dep: &str) -> Result<()> {
    if name == "dim-agent-types" && !modules::TYPES_VALUE_DEPENDENCIES.contains(&dep) {
        return Err("types must not depend on runtime/capabilities".into());
    }
    if name == "dim-agent-runtime" && (dep == "dim-agent-capabilities" || dep == "dim-app-server") {
        return Err("runtime must not depend on capabilities/server".into());
    }
    if name == "dim-agent-capabilities" && dep == "dim-app-server" {
        return Err("capabilities must not depend on server".into());
    }
    if name == "xtask-check" && (dep.starts_with("dim-") || dep == "rutis") {
        return Err(format!("xtask-check must stay free of business/runtime crate {dep}").into());
    }
    Ok(())
}

fn visit_dependencies(root: &Path, mut visit: impl FnMut(&str, &str) -> Result<()>) -> Result<()> {
    let output = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
        ])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy_owned(output.stderr).into());
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    for p in metadata["packages"].as_array().unwrap() {
        let name = p["name"].as_str().unwrap();
        for d in p["dependencies"].as_array().unwrap() {
            let dep = d["name"].as_str().unwrap();
            visit(name, dep)?;
        }
    }
    Ok(())
}

pub fn check(root: &Path) -> Result<()> {
    visit_dependencies(root, check_dependency)?;
    modules::check(root)?;
    println!("crate and module boundaries passed");
    Ok(())
}

pub fn write(root: &Path, out: &Path) -> Result<()> {
    let mut edges = Vec::new();
    let mut graph = String::from("%% Generated from cargo metadata; do not edit.\nflowchart LR\n");
    visit_dependencies(root, |name, dep| {
        if !dep.starts_with("dim-") && dep != "rutis" {
            return Ok(());
        }
        edges.push(json!({"from":name,"to":dep}));
        graph += &format!(
            "  {}[\"{name}\"] --> {}[\"{dep}\"]\n",
            name.replace('-', "_"),
            dep.replace('-', "_")
        );
        Ok(())
    })?;
    std::fs::write(
        out.join("crates.json"),
        serde_json::to_string_pretty(&edges)? + "\n",
    )?;
    std::fs::write(out.join("crates.mmd"), graph)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_dependency;

    #[test]
    fn xtask_check_rejects_business_and_runtime_crates() {
        assert!(check_dependency("xtask-check", "dim-agent-types").is_err());
        assert!(check_dependency("xtask-check", "dim-app-server").is_err());
        assert!(check_dependency("xtask-check", "rutis").is_err());
        assert!(check_dependency("xtask-check", "rutis-types").is_ok());
        assert!(check_dependency("xtask-check", "syn").is_ok());
    }
}
