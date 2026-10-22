//! Exports existing Session and branch services to TypeScript parts by name
//! (branch-typescript.md), generating their dispatch and declarations, then
//! builds the Web workspace (`web/`) and embeds it, gzip-compressed, in the
//! App Server binary. See docs/design/web-workspace.md §2.4.

use flate2::{Compression, write::GzEncoder};
use std::{
    fmt::Write as _,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

/// Already-compressed formats are stored as built.
const STORED: &[&str] = &[
    "woff2", "woff", "png", "jpg", "jpeg", "webp", "ico", "gz", "br",
];

fn main() {
    rutis_bridge::build::Exports::new()
        .service("sessionPermissions", "crate::agents::SessionPermissions")
        .service("workflowFiles", "crate::settings::WorkflowFiles")
        .service_in(
            "treeCommands",
            "dim_agent_capabilities::timeline_tree::TreeCommands",
            "session",
        )
        .service_in("dispatcher", "dim_agent_runtime::Dispatcher", "branch")
        .service_in(
            "branchLifecycle",
            "dim_agent_runtime::BranchLifecycle",
            "branch",
        )
        .service_in("timeline", "dim_agent_runtime::Timeline", "branch")
        .service_in(
            "timelineIdentity",
            "dim_agent_runtime::TimelineIdentity",
            "branch",
        )
        // Custom serde: reuse the SDK envelope with native bridge sequence values.
        .declared(
            "dim_agent_types::AgentEvent",
            "import('./plugin.js').BranchEvent",
        )
        // The same envelope before the Timeline assigns identity and order.
        .declared(
            "dim_agent_types::EventDraft",
            "{ type: string; name: string; data: unknown; tags?: string[] }",
        )
        // Reuse the identity envelope; the service keeps its own TypeScript name.
        .declared(
            "dim_agent_runtime::TimelineIdentity",
            "import('./plugin.js').SessionRef",
        )
        .typescript("../../npm/branch-services.d.ts")
        .generate()
        .expect("generate the exported services");
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let web = manifest.join("../../web").canonicalize().unwrap();
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    for input in [
        "src",
        "index.html",
        "vite.config.ts",
        "tsconfig.json",
        "package.json",
        "package-lock.json",
    ] {
        println!("cargo::rerun-if-changed={}", web.join(input).display());
    }
    let dist = out.join("web");
    let built = if npm(&web, &["--version"]).is_ok() {
        build(&web, &dist);
        true
    } else {
        println!(
            "cargo::warning=npm is not installed; the App Server is built without the Web workspace"
        );
        false
    };
    let mut files = Vec::new();
    if built {
        collect(&dist, &dist, &mut files);
    }
    files.sort();
    let mut table = String::from("&[\n");
    for (path, file) in files {
        let bytes = fs::read(&file).unwrap();
        let extension = file.extension().and_then(|e| e.to_str()).unwrap_or("");
        let gzip = !STORED.contains(&extension);
        let stored = out.join("web-embedded").join(&path[1..]);
        fs::create_dir_all(stored.parent().unwrap()).unwrap();
        if gzip {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
            encoder.write_all(&bytes).unwrap();
            fs::write(&stored, encoder.finish().unwrap()).unwrap();
        } else {
            fs::write(&stored, &bytes).unwrap();
        }
        writeln!(
            table,
            "    ({path:?}, {gzip}, include_bytes!({:?})),",
            stored.display().to_string()
        )
        .unwrap();
    }
    table.push(']');
    fs::write(out.join("web_assets.rs"), table).unwrap();
}

/// `npm ci` when the lockfile changed, then a production build into `dist`.
/// A lock file serializes concurrent builds sharing `web/node_modules`.
fn build(web: &Path, dist: &Path) {
    let lock = fs::File::create(web.join("node_modules.lock")).unwrap();
    lock.lock().unwrap();
    let installed = web.join("node_modules/.package-lock.json");
    let stale = fs::metadata(&installed)
        .and_then(|m| m.modified())
        .ok()
        .is_none_or(|installed| {
            fs::metadata(web.join("package-lock.json"))
                .and_then(|m| m.modified())
                .is_ok_and(|lock| lock > installed)
        });
    if stale {
        npm(web, &["ci", "--ignore-scripts", "--no-audit", "--no-fund"])
            .unwrap_or_else(|error| panic!("npm ci in web/ failed: {error}"));
    }
    let dist = dist.to_str().unwrap();
    npm(
        web,
        &["run", "build", "--", "--outDir", dist, "--emptyOutDir"],
    )
    .unwrap_or_else(|error| panic!("Web workspace build failed: {error}"));
}

fn npm(web: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" })
        .args(args)
        .current_dir(web)
        // Build output goes to cargo's log, not the protocol stdout of a
        // build script.
        .stdout(std::process::Stdio::from(std::io::stderr()))
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("npm {} exited with {status}", args.join(" ")))
}

fn collect(root: &Path, directory: &Path, files: &mut Vec<(String, PathBuf)>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            let relative = path.strip_prefix(root).unwrap().to_str().unwrap();
            files.push((format!("/{}", relative.replace('\\', "/")), path));
        }
    }
}
