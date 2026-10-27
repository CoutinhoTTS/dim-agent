//! `dimcode web install`: the Web workspace of this CLI's version from the
//! npm registry into `<home>/web`, verified against the registry's integrity
//! (web-workspace.md §2.4).

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use sha2::{Digest, Sha512};

const PACKAGE: &str = "dimcode-web";
const TIMEOUT: Duration = Duration::from_secs(120);

/// Installs `dimcode-web@<this version>` into `<home>/web`, replacing what
/// was there only once the new tree is complete.
pub async fn install(home: &Path) -> crate::Result<i32> {
    let version = env!("CARGO_PKG_VERSION");
    #[derive(serde::Deserialize)]
    struct Manifest {
        dist: Dist,
    }
    #[derive(serde::Deserialize)]
    struct Dist {
        tarball: String,
        integrity: String,
    }
    let client = reqwest::Client::builder().timeout(TIMEOUT).build()?;
    let registry = crate::update::registry();
    let manifest: Manifest = client
        .get(format!("{registry}/{PACKAGE}/{version}"))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("Cannot read {PACKAGE}@{version} from {registry}: {error}"))?
        .json()
        .await
        .map_err(|_| format!("Invalid {PACKAGE}@{version} metadata from {registry}"))?;
    let expected = manifest
        .dist
        .integrity
        .strip_prefix("sha512-")
        .and_then(|digest| {
            base64::engine::general_purpose::STANDARD
                .decode(digest)
                .ok()
        })
        .ok_or_else(|| format!("{PACKAGE}@{version} has no sha512 integrity"))?;
    println!("Downloading {PACKAGE}@{version}…");
    let tarball = client
        .get(&manifest.dist.tarball)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("Cannot download {}: {error}", manifest.dist.tarball))?
        .bytes()
        .await?;
    if Sha512::digest(&tarball).as_slice() != expected.as_slice() {
        return Err(format!("{PACKAGE}@{version} does not match its registry integrity").into());
    }
    let target = home.join("web");
    let partial = home.join("web.partial");
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial)?;
    if let Err(error) = unpack(&tarball, &partial) {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(error);
    }
    if !partial.join("index.html").is_file() {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(format!("{PACKAGE}@{version} has no web/index.html").into());
    }
    let previous = home.join("web.previous");
    let _ = std::fs::remove_dir_all(&previous);
    if target.exists() {
        std::fs::rename(&target, &previous)?;
    }
    std::fs::rename(&partial, &target)?;
    let _ = std::fs::remove_dir_all(&previous);
    println!("Web workspace {version} installed in {}.", target.display());
    let executable = std::env::current_exe().unwrap_or_default();
    if let Some(other) = crate::client::web_assets(&executable, home)
        .filter(|found| found.canonicalize().ok() != target.canonicalize().ok())
    {
        println!(
            "The Web workspace in {} is used instead; remove it to use this one.",
            other.display()
        );
    }
    Ok(0)
}

/// The package's `web/` files and directories under `into`; any other entry
/// is skipped, and a path leaving `web/` is refused.
fn unpack(tarball: &[u8], into: &Path) -> crate::Result<()> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tarball));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let Ok(relative) = path.strip_prefix("package/web") else {
            continue;
        };
        if !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(format!("Refusing archive path {}", path.display()).into());
        }
        let destination: PathBuf = into.join(relative);
        match entry.header().entry_type() {
            tar::EntryType::Directory => std::fs::create_dir_all(&destination)?,
            tar::EntryType::Regular => {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                entry.unpack(&destination)?;
            }
            _ => {}
        }
    }
    Ok(())
}
