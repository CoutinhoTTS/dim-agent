//! `write` (TS `write.ts`) and staged replacement shared with `edit`.

use super::super::file_reads::{Bom, EncodingSource, FileReads, TextEncoding, TextFileBinding};
use super::codec;
use super::{
    Cancelled, FileTools, NotRegular, blocking, cancelled, failed, field, hex, invalid_input,
    not_regular, open_options, open_regular, resolve_path, revision, succeeded,
};
use crate::host::HostError;
use dim_agent_types::{JsonObject, ToolResult};
use serde_json::{Value, json};
use std::{
    io::{ErrorKind, Read, Write},
    path::Path,
};
use tokio_util::sync::CancellationToken;

pub(super) fn definition() -> Value {
    json!({
        "name": "write",
        "description": "Write content to a file. Creates the file if it does not exist, or overwrites the entire content if it does.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to write." },
                "content": { "type": "string", "description": "Text content to write." },
            },
            "required": ["path", "content"],
        },
        "source": "builtin",
        "risk": "write_filesystem",
        "fileChanges": {
            "preExecutePaths": { "selector": "path" },
            "postExecutePaths": "path",
        },
    })
}

pub(super) async fn run(
    tools: &FileTools,
    input: &JsonObject,
    reads: &FileReads,
    cancel: CancellationToken,
) -> Result<ToolResult, HostError> {
    let Some(Value::String(display)) =
        field(input, "path").filter(|path| path.as_str() != Some(""))
    else {
        return Ok(invalid_input("write requires a non-empty string path."));
    };
    let Some(Value::String(content)) = field(input, "content") else {
        return Ok(invalid_input("write requires a string content."));
    };
    let (cwd, home, requested) = (tools.cwd.clone(), tools.home.clone(), display.clone());
    let path = match blocking(move || resolve_path(&cwd, &home, &requested)).await? {
        Ok(path) => path,
        Err(message) => return Ok(invalid_input(message)),
    };
    let _lock = tools.locks.lock(&path, &cancel).await?;
    let established = reads.get(&path);
    let (display, content, target) = (display.clone(), content.clone(), path.clone());
    #[cfg(test)]
    let hook = tools.before_commit.clone();
    let (_lock, written) = blocking(move || {
        let written = write_locked(
            &target,
            &display,
            &content,
            established,
            &cancel,
            #[cfg(test)]
            hook,
        );
        (_lock, written)
    })
    .await?;
    let (result, binding) = written.map_err(|Cancelled| cancelled())?;
    if let Some(binding) = binding {
        reads.set(path, binding);
    }
    Ok(result)
}

fn write_locked(
    path: &Path,
    display: &str,
    content: &str,
    established: Option<TextFileBinding>,
    cancel: &CancellationToken,
    #[cfg(test)] hook: Option<super::CommitHook>,
) -> Result<(ToolResult, Option<TextFileBinding>), Cancelled> {
    let done = |result| Ok((result, None));
    Cancelled::check(cancel)?;
    let exists = match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            return done(failed(
                format!("Cannot write to directory: {display}"),
                json!({ "code": "is_directory" }),
            ));
        }
        Ok(_) => true,
        Err(error) if error.kind() == ErrorKind::NotFound => false,
        Err(error) => {
            return done(failed(
                format!("Failed to stat file: {error}"),
                json!({ "code": "stat_error" }),
            ));
        }
    };
    Cancelled::check(cancel)?;
    let original = if exists {
        match open_regular(path) {
            Ok(Ok(file)) => Some(file),
            Ok(Err(NotRegular)) => return done(not_regular(display)),
            Err(error) => return done(write_failure(&error, display, false)),
        }
    } else {
        None
    };
    let (binding, text) = if exists {
        let Some(binding) = established else {
            return done(failed(
                "Read the existing file before overwriting it so its encoding and line endings can be preserved.",
                json!({ "code": "text_binding_required" }),
            ));
        };
        if let Some(conflict) = revision_conflict(path, &binding.raw_revision, display) {
            return done(conflict);
        }
        Cancelled::check(cancel)?;
        let text = codec::normalize_eol(content, binding.eol);
        (binding, text)
    } else {
        let binding = TextFileBinding {
            raw_revision: [0; 32],
            encoding: TextEncoding::Utf8,
            encoding_source: EncodingSource::Utf8Policy,
            bom: Bom::None,
            eol: codec::detect_eol(content),
        };
        (binding, content.to_owned())
    };
    let bytes = match codec::encode_text(&text, &binding) {
        Ok(bytes) => bytes,
        Err(error) => return done(error.into_result()),
    };
    Cancelled::check(cancel)?;
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return done(failed(
            format!("Failed to create parent directory: {error}"),
            json!({ "code": "mkdir_error" }),
        ));
    }
    Cancelled::check(cancel)?;
    let written = write_target(
        path,
        &bytes,
        display,
        original.as_ref(),
        &binding.raw_revision,
        cancel,
        #[cfg(test)]
        hook,
    )?;
    if let Err(result) = written {
        return done(result);
    }
    let raw_revision = revision(&bytes);
    let result = succeeded(
        format!("Wrote {display}"),
        json!({ "path": display, "bytesWritten": bytes.len(), "revision": hex(&raw_revision) }),
    );
    let binding = TextFileBinding {
        raw_revision,
        eol: codec::detect_eol(&text),
        ..binding
    };
    Ok((result, Some(binding)))
}

/// Fails closed: anything but the recorded bytes is a conflict.
fn revision_conflict(path: &Path, expected: &[u8; 32], display: &str) -> Option<ToolResult> {
    let modified = || {
        failed(
            "File was modified after it was last read. Use the read tool again before writing.",
            json!({ "code": "file_modified_since_read" }),
        )
    };
    let mut bytes = Vec::new();
    let read = match open_regular(path) {
        Ok(Ok(mut file)) => file.read_to_end(&mut bytes),
        Ok(Err(NotRegular)) => return Some(not_regular(display)),
        Err(error) => Err(error),
    };
    match read {
        Ok(_) if revision(&bytes) == *expected => None,
        Err(error) if error.kind() == ErrorKind::PermissionDenied => Some(failed(
            format!("Permission denied: {display}"),
            json!({ "code": "permission_denied" }),
        )),
        _ => Some(modified()),
    }
}

/// Stages complete bytes before touching the destination. Verification is not
/// portable CAS: uncoordinated path/parent renames or metadata changes can still
/// race the final syscall. Windows installs the owned handle, not a temp name.
pub(super) fn write_target(
    path: &Path,
    bytes: &[u8],
    display: &str,
    original: Option<&std::fs::File>,
    expected: &[u8; 32],
    cancel: &CancellationToken,
    #[cfg(test)] hook: Option<super::CommitHook>,
) -> Result<Result<(), ToolResult>, Cancelled> {
    if let Some(original) = original {
        let mut options = open_options();
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::custom_flags(
            &mut options,
            libc::O_NOFOLLOW | libc::O_NONBLOCK,
        );
        let writable = match options.write(true).open(path) {
            Ok(file) => file,
            Err(error) => return Ok(Err(write_failure(&error, display, false))),
        };
        if !same_identity(original, &writable).unwrap_or(false) {
            return Ok(Err(failed(
                "File was replaced during preparation. Read it again.",
                json!({"code": "file_modified_since_read"}),
            )));
        }
    }
    let metadata_before = match original.map(metadata_revision).transpose() {
        Ok(value) => value,
        Err(error) => return Ok(Err(write_failure(&error, display, false))),
    };
    #[cfg(windows)]
    if original.is_some_and(|file| file.metadata().is_ok_and(|m| m.permissions().readonly())) {
        return Ok(Err(write_failure(
            &ErrorKind::PermissionDenied.into(),
            display,
            false,
        )));
    }
    #[cfg(windows)]
    let parent = match crate::file_storage::windows::open_root(path.parent().unwrap()) {
        Ok(parent) => parent,
        Err(error) => {
            return Ok(Err(write_failure(
                &std::io::Error::other(error),
                display,
                false,
            )));
        }
    };
    struct Temporary(std::path::PathBuf, std::fs::File);
    impl Drop for Temporary {
        fn drop(&mut self) {
            if owned_temporary(&self.0, &self.1) {
                #[cfg(windows)]
                if let Ok(metadata) = self.1.metadata() {
                    let mut permissions = metadata.permissions();
                    permissions.set_readonly(false);
                    let _ = self.1.set_permissions(permissions);
                }
                let _ = std::fs::remove_file(&self.0);
            }
        }
    }
    let staged = path.with_file_name(format!(".dim-write-{}", uuid::Uuid::new_v4()));
    let mut options = open_options();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    #[cfg(unix)]
    let file = options.open(&staged);
    #[cfg(windows)]
    let file = crate::file_storage::windows::create_temp(
        &parent,
        &staged.file_name().unwrap().to_owned(),
        &parent,
        &path.file_name().unwrap().to_owned(),
        None,
    );
    let file = match file {
        Ok(file) => file,
        Err(error) => return Ok(Err(write_failure(&error, display, false))),
    };
    let mut temporary = Temporary(staged, file);
    let prepare = (|| -> std::io::Result<()> {
        temporary.1.write_all(bytes)?;
        if let Some(original) = original {
            #[cfg(unix)]
            {
                use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
                let metadata = original.metadata()?;
                let staged = temporary.1.metadata()?;
                if (metadata.uid(), metadata.gid()) != (staged.uid(), staged.gid())
                    && unsafe {
                        libc::fchown(temporary.1.as_raw_fd(), metadata.uid(), metadata.gid())
                    } != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            }
            temporary
                .1
                .set_permissions(original.metadata()?.permissions())?;
            preserve_metadata(original, &temporary.1)?;
        }
        temporary.1.sync_all()
    })();
    if let Err(error) = prepare {
        return Ok(Err(write_failure(&error, display, false)));
    }
    #[cfg(test)]
    if let Some(hook) = hook {
        hook(path);
    }
    Cancelled::check(cancel)?;
    if let Some(original) = original {
        if let Some(conflict) = revision_conflict(path, expected, display) {
            return Ok(Err(conflict));
        }
        let same = open_regular(path)
            .and_then(|current| match current {
                Ok(current) => same_identity(original, &current),
                Err(_) => Ok(false),
            })
            .unwrap_or(false)
            && metadata_revision(original).ok().as_ref() == metadata_before.as_ref()
            && std::fs::symlink_metadata(path)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.is_symlink())
            && original
                .metadata()
                .and_then(|metadata| {
                    temporary
                        .1
                        .metadata()
                        .map(|staged| metadata.permissions() == staged.permissions())
                })
                .unwrap_or(false);
        if !same {
            return Ok(Err(failed(
                "File was replaced during preparation. Read it again.",
                json!({"code": "file_modified_since_read"}),
            )));
        }
        #[cfg(windows)]
        if metadata_revision(&temporary.1).ok().as_ref() != metadata_before.as_ref() {
            return Ok(Err(failed(
                "File security changed during preparation.",
                json!({"code": "file_modified_since_read"}),
            )));
        }
    }
    Cancelled::check(cancel)?;
    if !owned_temporary(&temporary.0, &temporary.1) {
        return Ok(Err(failed(
            "Staged file was replaced during preparation.",
            json!({"code": "write_error"}),
        )));
    }
    // No cancellation after the commit point. hard_link publishes an absent
    // target without replacing a name created by another writer.
    let committed = if original.is_some() {
        #[cfg(unix)]
        let result = std::fs::rename(&temporary.0, path);
        #[cfg(windows)]
        let result = crate::file_storage::windows::replace(
            &temporary.1,
            &parent,
            &temporary.0.file_name().unwrap().to_owned(),
            &parent,
            &path.file_name().unwrap().to_owned(),
        );
        result
    } else {
        std::fs::hard_link(&temporary.0, path)
    };
    if let Err(error) = committed {
        return Ok(Err(
            if original.is_none() && error.kind() == ErrorKind::AlreadyExists {
                failed(
                    "Read the existing file before overwriting it.",
                    json!({"code": "text_binding_required"}),
                )
            } else {
                write_failure(&error, display, false)
            },
        ));
    }
    drop(temporary);
    #[cfg(unix)]
    if let Err(error) = std::fs::File::open(path.parent().expect("resolved target has parent"))
        .and_then(|parent| parent.sync_all())
    {
        return Ok(Err(write_failure(&error, display, true)));
    }
    Ok(Ok(()))
}

#[cfg(unix)]
fn metadata_revision(file: &std::fs::File) -> std::io::Result<Vec<u8>> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    Ok(format!(
        "{}:{}:{}:{}:{}",
        m.ctime(),
        m.ctime_nsec(),
        m.uid(),
        m.gid(),
        m.mode()
    )
    .into_bytes())
}

#[cfg(windows)]
fn metadata_revision(file: &std::fs::File) -> std::io::Result<Vec<u8>> {
    crate::file_storage::windows::workspace_security_revision(file)
}

fn preserve_metadata(source: &std::fs::File, target: &std::fs::File) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        if unsafe {
            libc::fcopyfile(
                source.as_raw_fd(),
                target.as_raw_fd(),
                std::ptr::null_mut(),
                libc::COPYFILE_METADATA,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        // A sibling can inherit a default ACL or security labels which the
        // source did not have. Copy the exact set, not their union.
        let target_fd = target.as_raw_fd();
        let size = unsafe { libc::flistxattr(target_fd, std::ptr::null_mut(), 0) };
        if size < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut inherited = vec![0u8; (size as usize).max(1)];
        let size =
            unsafe { libc::flistxattr(target_fd, inherited.as_mut_ptr().cast(), inherited.len()) };
        if size < 0 {
            return Err(std::io::Error::last_os_error());
        }
        for name in inherited[..size as usize].split_inclusive(|b| *b == 0) {
            if unsafe { libc::fremovexattr(target_fd, name.as_ptr().cast()) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        let fd = source.as_raw_fd();
        let size = unsafe { libc::flistxattr(fd, std::ptr::null_mut(), 0) };
        if size < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut names = vec![0u8; (size as usize).max(1)];
        let size = unsafe { libc::flistxattr(fd, names.as_mut_ptr().cast(), names.len()) };
        if size < 0 {
            return Err(std::io::Error::last_os_error());
        }
        for name in names[..size as usize].split_inclusive(|b| *b == 0) {
            let name = name.as_ptr().cast();
            let size = unsafe { libc::fgetxattr(fd, name, std::ptr::null_mut(), 0) };
            if size < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut value = vec![0u8; (size as usize).max(1)];
            let size = unsafe { libc::fgetxattr(fd, name, value.as_mut_ptr().cast(), value.len()) };
            if size < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if unsafe {
                libc::fsetxattr(
                    target.as_raw_fd(),
                    name,
                    value.as_ptr().cast(),
                    size as usize,
                    0,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
    }
    #[cfg(windows)]
    let _ = (source, target); // DACL is already installed at creation.
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    return Err(std::io::ErrorKind::Unsupported.into());
    Ok(())
}

fn same_identity(a: &std::fs::File, b: &std::fs::File) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (a, b) = (a.metadata()?, b.metadata()?);
        Ok(a.dev() == b.dev() && a.ino() == b.ino())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let info = |file: &std::fs::File| -> std::io::Result<BY_HANDLE_FILE_INFORMATION> {
            let mut info = unsafe { std::mem::zeroed() };
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(info)
        };
        let (a, b) = (info(a)?, info(b)?);
        Ok(a.dwVolumeSerialNumber == b.dwVolumeSerialNumber
            && a.nFileIndexHigh == b.nFileIndexHigh
            && a.nFileIndexLow == b.nFileIndexLow)
    }
}

fn owned_temporary(path: &Path, owned: &std::fs::File) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (owned.metadata(), std::fs::symlink_metadata(path)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    #[cfg(windows)]
    {
        open_regular(path)
            .ok()
            .and_then(Result::ok)
            .is_some_and(|current| same_identity(owned, &current).unwrap_or(false))
    }
}

fn write_failure(error: &std::io::Error, display: &str, may_have_modified: bool) -> ToolResult {
    let warning = if may_have_modified {
        "\nThe file may have been partially written. Read it before retrying."
    } else {
        ""
    };
    let (code, message) = if is_locked(error) {
        (
            "file_locked",
            format!(
                "File is locked by another process: {display}. Close the process that is using it and retry.{warning}"
            ),
        )
    } else if error.kind() == ErrorKind::PermissionDenied {
        (
            "permission_denied",
            format!("Permission denied: {display}{warning}"),
        )
    } else if error.kind() == ErrorKind::StorageFull {
        (
            "no_space_left",
            format!("No space left on device: {display}.{warning}"),
        )
    } else {
        (
            "write_error",
            format!("Failed to write file: {error}.{warning}"),
        )
    };
    let mut structured = json!({ "code": code });
    if may_have_modified {
        structured["mayHaveModified"] = json!(true);
    }
    failed(message, structured)
}

fn is_locked(error: &std::io::Error) -> bool {
    // ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION, which Node reports as EBUSY.
    error.kind() == ErrorKind::ResourceBusy
        || (cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)))
}
