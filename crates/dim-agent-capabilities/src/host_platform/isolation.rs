//! Node-local policy at the model process boundary, before ordinary spawn/PTY.
use super::{HostPlatform, ProcessSpec, ProcessSpecError};
use crate::file_storage::FileStorageError;
use serde::Deserialize;
use std::path::Path;
use std::{ffi::OsString, path::PathBuf};
use tokio_util::sync::CancellationToken;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Policy {
    version: u8,
    enabled: Option<bool>,
    #[serde(default)]
    network: bool,
    read_only_paths: Option<Vec<PathBuf>>,
    writable_paths: Option<Vec<PathBuf>>,
    environment: Option<Vec<String>>,
}

async fn roots(paths: Vec<PathBuf>) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut resolved = Vec::with_capacity(paths.len());
    for path in paths {
        if !path.is_absolute() {
            return Err(format!(
                "Isolation root must be absolute: {}",
                path.display()
            ));
        }
        let canonical = tokio::fs::canonicalize(&path)
            .await
            .map_err(|error| format!("Isolation root {}: {error}", path.display()))?;
        if !tokio::fs::metadata(&canonical)
            .await
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            return Err(format!(
                "Isolation root must be a directory: {}",
                path.display()
            ));
        }
        resolved.push((path, canonical));
    }
    resolved.sort_by_key(|(path, _)| path.components().count());
    Ok(resolved)
}

impl HostPlatform {
    pub(crate) async fn prepare_isolated_process(
        &self,
        mut spec: ProcessSpec,
        workspace: &Path,
        output_dir: Option<&Path>,
    ) -> Result<ProcessSpec, ProcessSpecError> {
        let invalid = |reason| ProcessSpecError {
            code: "isolation_config_invalid",
            reason,
        };
        let bytes = match &self.isolation_file {
            Some((storage, root, relative)) => {
                let path = storage
                    .path(*root, relative)
                    .map_err(|e| invalid(e.to_string()))?;
                match storage.read(&path, &CancellationToken::new()).await {
                    Ok(bytes) => Some(bytes),
                    Err(FileStorageError::Io(error))
                        if error.kind() == std::io::ErrorKind::NotFound =>
                    {
                        None
                    }
                    Err(error) => return Err(invalid(error.to_string())),
                }
            }
            None => None,
        };
        let policy: Policy = match bytes {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                invalid(format!(
                    "Invalid host-isolation.json at line {}, column {}",
                    error.line(),
                    error.column()
                ))
            })?,
            None => Policy {
                version: 1,
                ..Policy::default()
            },
        };
        if policy.version != 1 {
            return Err(invalid("host-isolation.json version must be 1".into()));
        }
        let enabled = policy.enabled != Some(false);
        let environment = policy.environment.unwrap_or_else(|| {
            [
                "PATH",
                "HOME",
                "LANG",
                "LC_ALL",
                "LC_CTYPE",
                "TERM",
                "NO_COLOR",
                "PAGER",
                "GIT_PAGER",
                "GH_PAGER",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        });
        if environment
            .iter()
            .any(|name| name.is_empty() || name.contains(['\0', '=']))
        {
            return Err(invalid(
                "Environment names must be nonempty and contain neither NUL nor '='".into(),
            ));
        }
        let read_only_paths = match policy.read_only_paths {
            Some(paths) => paths,
            None if !enabled => Vec::new(),
            None => {
                #[cfg(target_os = "macos")]
                let paths = [
                    "/System",
                    "/usr",
                    "/bin",
                    "/sbin",
                    "/Library/Apple",
                    "/private/etc",
                    "/etc",
                ];
                #[cfg(not(target_os = "macos"))]
                let paths = ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"];
                let mut installed = Vec::new();
                for path in paths {
                    match tokio::fs::metadata(path).await {
                        Ok(_) => installed.push(path.into()),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(invalid(format!("System isolation root {path}: {error}")));
                        }
                    }
                }
                installed
            }
        };
        let read_only = roots(read_only_paths).await.map_err(invalid)?;
        let writable_paths = policy.writable_paths.unwrap_or_else(|| {
            if enabled {
                std::iter::once(workspace)
                    .chain(output_dir)
                    .map(Path::to_path_buf)
                    .collect()
            } else {
                Vec::new()
            }
        });
        let writable = roots(writable_paths).await.map_err(invalid)?;
        if read_only
            .iter()
            .any(|(_, read)| writable.iter().any(|(_, write)| read.starts_with(write)))
        {
            return Err(invalid(
                "A resolved read-only isolation root cannot equal or be inside a writable root"
                    .into(),
            ));
        }
        if !enabled {
            return Ok(spec);
        }
        spec.env
            .retain(|(name, _)| environment.iter().any(|allowed| name == allowed.as_str()));
        prepare(self, spec, &read_only, &writable, policy.network)
    }
}

#[cfg(target_os = "macos")]
fn prepare(
    _host: &HostPlatform,
    mut spec: ProcessSpec,
    read_only: &[(PathBuf, PathBuf)],
    writable: &[(PathBuf, PathBuf)],
    network: bool,
) -> Result<ProcessSpec, ProcessSpecError> {
    let mut profile = String::from(
        "(version 1)(deny default)(allow process*)(allow sysctl-read)\
         (allow file-read* (literal \"/\") (literal \"/dev/random\") (literal \"/dev/urandom\"))\
         (allow file-read* file-write* (literal \"/dev/null\"))",
    );
    // Inherited PTYs need terminal control without opening other devices.
    for command in [
        libc::TIOCGETD,
        libc::TIOCGETA,
        libc::TIOCSETA,
        libc::TIOCSETAW,
        libc::TIOCSETAF,
        libc::TIOCGWINSZ,
        libc::TIOCSWINSZ,
        libc::TIOCGPGRP,
        libc::TIOCSPGRP,
    ] {
        profile.push_str(&format!(
            "(allow file-ioctl (require-all (regex #\"^/dev/(ttys[0-9]+|tty)$\") (ioctl-command {command})))"
        ));
    }
    let mut args: Vec<OsString> = Vec::new();
    for (index, ((destination, source), write)) in read_only
        .iter()
        .map(|root| (root, false))
        .chain(writable.iter().map(|root| (root, true)))
        .enumerate()
    {
        for (alias, path) in std::iter::once(source)
            .chain((destination != source).then_some(destination))
            .enumerate()
        {
            let name = format!("ROOT_{index}_{alias}");
            let mut parameter = OsString::from(format!("{name}="));
            parameter.push(path);
            args.extend(["-D".into(), parameter]);
            profile.push_str(&format!(
                "(allow file-read* {}(subpath (param \"{name}\")))",
                if write { "file-write* " } else { "" }
            ));
            if network {
                profile.push_str(&format!(
                    "(allow network-outbound (remote unix-socket (subpath (param \"{name}\"))))"
                ));
                if write {
                    profile.push_str(&format!(
                        "(allow network-bind network-inbound (local unix-socket (subpath (param \"{name}\"))))"
                    ));
                }
            }
        }
    }
    if network {
        profile.push_str(
            "(allow network* (local ip) (remote ip))\
             (allow file-read-metadata (literal \"/var\"))\
             (allow network-outbound (literal \"/private/var/run/mDNSResponder\"))",
        );
    }
    args.extend([
        "-p".into(),
        profile.into(),
        spec.executable.into_os_string(),
    ]);
    args.append(&mut spec.args);
    spec.executable = "/usr/bin/sandbox-exec".into();
    spec.args = args;
    Ok(spec)
}

#[cfg(not(target_os = "macos"))]
fn prepare(
    host: &HostPlatform,
    mut spec: ProcessSpec,
    read_only: &[(PathBuf, PathBuf)],
    writable: &[(PathBuf, PathBuf)],
    network: bool,
) -> Result<ProcessSpec, ProcessSpecError> {
    use std::os::unix::fs::PermissionsExt;
    let executable = super::path_entries(&host.env)
        .into_iter()
        .map(|path| path.join("bwrap"))
        .find(|path| {
            std::fs::metadata(path)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .ok_or_else(|| ProcessSpecError {
            code: "isolation_unavailable",
            reason: "bubblewrap is not installed on the captured host PATH".into(),
        })?;
    let mut args: Vec<OsString> = [
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--die-with-parent",
        "--cap-drop",
        "ALL",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    if !network {
        args.push("--unshare-net".into());
    }
    for (flag, paths) in [("--ro-bind", read_only), ("--bind", writable)] {
        for (destination, source) in paths {
            args.extend([
                flag.into(),
                source.as_os_str().into(),
                destination.as_os_str().into(),
            ]);
        }
    }
    args.extend([
        "--proc".into(),
        "/proc".into(),
        "--dev".into(),
        "/dev".into(),
        "--remount-ro".into(),
        "/".into(),
        "--chdir".into(),
        spec.cwd.as_os_str().into(),
        "--".into(),
        spec.executable.into_os_string(),
    ]);
    args.append(&mut spec.args);
    spec.executable = executable;
    spec.args = args;
    Ok(spec)
}
