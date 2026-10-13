//! `write` (TS `write.ts`) and the in-place write shared with `edit`.

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
    let (result, binding) =
        blocking(move || write_locked(&target, &display, &content, established, &cancel))
            .await?
            .map_err(|Cancelled| cancelled())?;
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
    // Writing has begun: cancellation is no longer observed. A file that
    // appeared since the check is not ours to overwrite.
    let written = if exists {
        write_target(path, &bytes, display)
    } else {
        create_target(path, &bytes, display)
    };
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

/// Writes the target in place, so Windows does not require delete sharing
/// from processes that already have the file open.
pub(super) fn write_target(path: &Path, bytes: &[u8], display: &str) -> Result<(), ToolResult> {
    let mut file = open_options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|error| write_failure(&error, display, false))?;
    file.write_all(bytes)
        .map_err(|error| write_failure(&error, display, true))
}

/// Creates a file that did not exist; one created meanwhile is refused.
fn create_target(path: &Path, bytes: &[u8], display: &str) -> Result<(), ToolResult> {
    let mut file = open_options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == ErrorKind::AlreadyExists {
                failed(
                    "Read the existing file before overwriting it so its encoding and line endings can be preserved.",
                    json!({ "code": "text_binding_required" }),
                )
            } else {
                write_failure(&error, display, false)
            }
        })?;
    file.write_all(bytes)
        .map_err(|error| write_failure(&error, display, true))
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
