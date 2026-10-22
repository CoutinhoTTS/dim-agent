//! The account runtime an account build carries inside its executable
//! (scripts/stage-runtime.mjs): the target's Node, the rutis runtime and the
//! dim-oauth plugin. It is extracted once per archive to
//! `<product home>/runtime/<digest>`, laid out as `runtime/node`,
//! `runtime/rutis` and `plugins/dim-oauth`.

use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

static ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/runtime.bin"));
const DIGEST: &str = include_str!(concat!(env!("OUT_DIR"), "/runtime.sha256"));
const MAGIC: &[u8] = b"DIMRT1\n";
/// Written last: a directory without it was never completely extracted.
const COMPLETE: &str = ".complete";
const PARTIAL: &str = ".partial-";

/// The extracted runtime's root, or `None` when this build embeds none.
pub fn root(home: &Path) -> io::Result<Option<PathBuf>> {
    if ARCHIVE.is_empty() {
        return Ok(None);
    }
    let base = home.join("runtime");
    let root = base.join(&DIGEST[..16]);
    if root.join(COMPLETE).is_file() {
        return Ok(Some(root));
    }
    fs::create_dir_all(&base)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let partial = base.join(format!("{PARTIAL}{}-{nanos}", std::process::id()));
    let extracted = extract(ARCHIVE, &partial)
        .and_then(|()| fs::write(partial.join(COMPLETE), DIGEST))
        .and_then(|()| fs::rename(&partial, &root));
    if let Err(error) = extracted {
        let _ = fs::remove_dir_all(&partial);
        // Another process may have extracted the same archive first.
        if !root.join(COMPLETE).is_file() {
            return Err(error);
        }
    }
    // Runtimes of earlier versions are no longer used by this executable.
    for entry in fs::read_dir(&base)?.flatten() {
        let name = entry.file_name();
        if entry.path() != root && !name.to_string_lossy().starts_with(PARTIAL) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(Some(root))
}

/// Writes the archive's files under `into`, rejecting any path that would
/// leave it.
fn extract(archive: &[u8], into: &Path) -> io::Result<()> {
    let invalid = |what: &str| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid embedded runtime: {what}"),
        )
    };
    let mut data = Vec::new();
    flate2::read::GzDecoder::new(archive).read_to_end(&mut data)?;
    let mut rest = data.strip_prefix(MAGIC).ok_or_else(|| invalid("header"))?;
    fs::create_dir_all(into)?;
    while !rest.is_empty() {
        let length = u32::from_be_bytes(take(&mut rest, 4)?.try_into().expect("4 bytes")) as usize;
        let name = std::str::from_utf8(take(&mut rest, length)?).map_err(|_| invalid("path"))?;
        let executable = take(&mut rest, 1)?[0] & 1 == 1;
        let size = u64::from_be_bytes(take(&mut rest, 8)?.try_into().expect("8 bytes"));
        let bytes = take(
            &mut rest,
            usize::try_from(size).map_err(|_| invalid("size"))?,
        )?;
        let relative = Path::new(name);
        if name.is_empty()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(invalid("path"));
        }
        let path = into.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if executable { 0o755 } else { 0o644 };
            fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
        }
        #[cfg(not(unix))]
        let _ = executable;
    }
    Ok(())
}

/// The next `count` bytes of the archive.
fn take<'a>(rest: &mut &'a [u8], count: usize) -> io::Result<&'a [u8]> {
    if rest.len() < count {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid embedded runtime: truncated entry",
        ));
    }
    let (taken, remaining) = rest.split_at(count);
    *rest = remaining;
    Ok(taken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut raw = MAGIC.to_vec();
        for (name, bytes) in entries {
            raw.extend((name.len() as u32).to_be_bytes());
            raw.extend(name.as_bytes());
            raw.push(1);
            raw.extend((bytes.len() as u64).to_be_bytes());
            raw.extend(*bytes);
        }
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn extraction_writes_the_tree_and_refuses_paths_outside_it() {
        let directory = tempfile::tempdir().unwrap();
        let into = directory.path().join("root");
        extract(
            &archive(&[
                ("runtime/node/bin/node", b"node"),
                ("plugins/dim-oauth/dist/index.js", b"js"),
            ]),
            &into,
        )
        .unwrap();
        assert_eq!(
            fs::read(into.join("runtime/node/bin/node")).unwrap(),
            b"node"
        );
        assert_eq!(
            fs::read(into.join("plugins/dim-oauth/dist/index.js")).unwrap(),
            b"js"
        );
        for name in ["../escape", "/absolute", "a/../../escape", ""] {
            let target = directory.path().join(format!("bad-{}", name.len()));
            assert!(
                extract(&archive(&[(name, b"x")]), &target).is_err(),
                "{name:?}"
            );
        }
        assert!(!directory.path().join("escape").exists());
    }
}
