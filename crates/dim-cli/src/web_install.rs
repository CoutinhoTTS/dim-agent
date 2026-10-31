//! `dimcode web install`: the Web workspace of this build's version from the
//! npm registry into `<home>/web`, verified against the registry's integrity
//! (web-workspace.md §2.4).

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use sha2::{Digest, Sha512};

const PACKAGE: &str = "dimcode-web";
const TIMEOUT: Duration = Duration::from_secs(120);

/// Installs `dimcode-web@version` from `registry` into `<home>/web`, replacing
/// what was there only once the new tree is complete, and reports to `out`.
/// `assets` is the frontend the caller found before installing: one in an
/// earlier source still comes first, and the report names it.
pub async fn install(
    home: &Path,
    version: &str,
    registry: &str,
    assets: Option<PathBuf>,
    out: &mut impl Write,
) -> crate::Result<i32> {
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
    writeln!(out, "Downloading {PACKAGE}@{version}…")?;
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
    writeln!(
        out,
        "Web workspace {version} installed in {}.",
        target.display()
    )?;
    if let Some(other) =
        assets.filter(|found| found.canonicalize().ok() != target.canonicalize().ok())
    {
        writeln!(
            out,
            "The Web workspace in {} is used instead; remove it to use this one.",
            other.display()
        )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use sha2::Digest;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    /// A gzip'd `dimcode-web` tarball holding `web/index.html` and
    /// `web/assets/app-1.js` beside a package manifest outside `web/`.
    fn tarball() -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        for (path, contents) in [
            ("package/package.json", "{}"),
            ("package/web/index.html", "<!doctype html>"),
            ("package/web/assets/app-1.js", "export {}"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o644);
            header.set_size(contents.len() as u64);
            builder
                .append_data(&mut header, path, contents.as_bytes())
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn integrity(tarball: &[u8]) -> String {
        format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(Sha512::digest(tarball))
        )
    }

    /// A registry serving `version`'s metadata and one tarball; its base URL.
    fn registry(version: &'static str, tarball: Vec<u8>, integrity: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let manifest = serde_json::json!({
            "dist": {"tarball": format!("{base}/dimcode-web.tgz"), "integrity": integrity}
        })
        .to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut wire = BufReader::new(stream.unwrap());
                let mut request = String::new();
                wire.read_line(&mut request).unwrap();
                let mut line = String::new();
                while wire.read_line(&mut line).unwrap() > 2 {
                    line.clear();
                }
                let body = if request.starts_with(&format!("GET /dimcode-web/{version} ")) {
                    manifest.as_bytes().to_vec()
                } else {
                    assert!(request.starts_with("GET /dimcode-web.tgz "), "{request}");
                    tarball.clone()
                };
                let stream = wire.get_mut();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        base
    }

    #[tokio::test]
    async fn install_puts_the_requested_versions_frontend_in_the_home() {
        let tarball = tarball();
        let base = registry("0.1.0-alpha.8", tarball.clone(), integrity(&tarball));
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("web")).unwrap();
        std::fs::write(home.path().join("web/stale.js"), "").unwrap();

        let mut out = Vec::new();
        assert_eq!(
            install(home.path(), "0.1.0-alpha.8", &base, None, &mut out)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!(
                "Downloading dimcode-web@0.1.0-alpha.8…\nWeb workspace 0.1.0-alpha.8 installed in {}.\n",
                home.path().join("web").display()
            )
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join("web/assets/app-1.js")).unwrap(),
            "export {}"
        );
        assert!(home.path().join("web/index.html").is_file());
        assert!(!home.path().join("web/stale.js").exists());
        assert!(!home.path().join("web/package.json").exists());
        assert!(!home.path().join("web.partial").exists());
    }

    #[tokio::test]
    async fn install_names_a_frontend_in_an_earlier_source() {
        let tarball = tarball();
        let base = registry("0.1.0-alpha.8", tarball.clone(), integrity(&tarball));
        let home = tempfile::tempdir().unwrap();
        let first = home.path().join("npm-web");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::write(first.join("index.html"), "<!doctype html>").unwrap();

        let mut out = Vec::new();
        assert_eq!(
            install(
                home.path(),
                "0.1.0-alpha.8",
                &base,
                Some(first.clone()),
                &mut out
            )
            .await
            .unwrap(),
            0
        );
        assert!(String::from_utf8(out).unwrap().ends_with(&format!(
            "The Web workspace in {} is used instead; remove it to use this one.\n",
            first.display()
        )));
    }

    #[tokio::test]
    async fn install_rejects_a_tarball_that_does_not_match_its_integrity() {
        let tarball = tarball();
        let base = registry(
            "0.1.0-alpha.8",
            tarball.clone(),
            format!("sha512-{}", "A".repeat(88)),
        );
        let home = tempfile::tempdir().unwrap();
        let error = install(home.path(), "0.1.0-alpha.8", &base, None, &mut Vec::new())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match its registry integrity"),
            "{error}"
        );
        assert!(!home.path().join("web").exists());
    }
}
