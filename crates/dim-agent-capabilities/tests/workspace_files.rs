//! File tool flows: text preservation, branch-scoped read bindings, write
//! serialization, and glob/grep parity with the TS grep-glob acceptance test.

use dim_agent_capabilities::{
    host::HostError,
    workspace_tools::{file_reads::FileReads, files::FileTools},
};
use dim_agent_runtime::{MemoryTimelineStore, Timeline, TimelineKey};
use dim_agent_types::{ToolCall, ToolContent, ToolResult};
use rutis::host::CtxHost;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

/// One branch with its own empty Timeline.
fn branch() -> FileReads {
    FileReads::new(Arc::new(Timeline::new(
        rutis::Ctx::root().unwrap(),
        TimelineKey {
            session_id: "workspace".into(),
            branch_id: "main".into(),
        },
        Vec::new(),
        Arc::new(MemoryTimelineStore::default()),
    )))
}

struct Workspace {
    root: PathBuf,
    tools: FileTools,
}
impl Workspace {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .join(format!("dim-workspace-files-{}", uuid::Uuid::new_v4()))
            .join("work");
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let home = root.parent().unwrap().to_owned();
        Self {
            tools: FileTools::new(root.clone(), home.clone(), home.join("outputs")),
            root,
        }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    fn put(&self, name: &str, bytes: &[u8]) {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fn bytes(&self, name: &str) -> Vec<u8> {
        fs::read(self.path(name)).unwrap()
    }
    async fn call(&self, reads: &FileReads, name: &str, input: Value) -> ToolResult {
        self.try_call(reads, name, input, CancellationToken::new())
            .await
            .unwrap()
    }
    async fn try_call(
        &self,
        reads: &FileReads,
        name: &str,
        input: Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, HostError> {
        let call = ToolCall {
            id: "call".into(),
            name: name.into(),
            input: input.as_object().unwrap().clone(),
        };
        self.tools.execute(&call, reads, cancel).await
    }
    async fn read(&self, reads: &FileReads, name: &str) -> ToolResult {
        self.call(reads, "read", json!({ "path": name })).await
    }
    async fn edit(&self, reads: &FileReads, name: &str, old: &str, new: &str) -> ToolResult {
        self.call(
            reads,
            "edit",
            json!({ "path": name, "oldText": old, "newText": new }),
        )
        .await
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.root.parent().unwrap());
    }
}

fn text(result: &ToolResult) -> &str {
    match &result.content {
        ToolContent::Text(text) => text,
        other => panic!("unexpected content {other:?}"),
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn atomic_mutations_preserve_native_metadata() {
    use std::{os::unix::fs::MetadataExt, process::Command};
    for tool in ["write", "edit"] {
        let workspace = Workspace::new();
        workspace.put("file", b"before\n");
        let path = workspace.path("file");
        assert!(
            Command::new("xattr")
                .args(["-w", "com.dim.regression", "private"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        fs::write(path.join("..namedfork/rsrc"), b"resource fork").unwrap();
        let user =
            String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout).unwrap();
        assert!(
            Command::new("chmod")
                .arg("+a")
                .arg(format!(
                    "{} allow read,write,readattr,writeattr,readextattr,writeextattr,readsecurity",
                    user.trim()
                ))
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let acl = Command::new("ls")
            .arg("-le")
            .arg(&path)
            .output()
            .unwrap()
            .stdout;
        let acl = String::from_utf8(acl)
            .unwrap()
            .lines()
            .skip(1)
            .collect::<Vec<_>>()
            .join("\n");
        let before = fs::metadata(&path).unwrap();
        let reads = branch();
        workspace.read(&reads, "file").await;
        let result = workspace.call(&reads, tool, json!({"path": "file", "content": "after\n", "oldText": "before", "newText": "after"})).await;
        assert_eq!(result.is_error, None, "{}", text(&result));
        assert_eq!(workspace.bytes("file"), b"after\n");
        assert_eq!(
            fs::read(path.join("..namedfork/rsrc")).unwrap(),
            b"resource fork"
        );
        assert_eq!(
            Command::new("xattr")
                .args(["-p", "com.dim.regression"])
                .arg(&path)
                .output()
                .unwrap()
                .stdout,
            b"private\n"
        );
        let after_acl = Command::new("ls")
            .arg("-le")
            .arg(&path)
            .output()
            .unwrap()
            .stdout;
        assert_eq!(
            String::from_utf8(after_acl)
                .unwrap()
                .lines()
                .skip(1)
                .collect::<Vec<_>>()
                .join("\n"),
            acl
        );
        let after = fs::metadata(&path).unwrap();
        assert_eq!(
            (after.uid(), after.gid(), after.mode()),
            (before.uid(), before.gid(), before.mode())
        );
    }
}
fn structured(result: &ToolResult) -> &serde_json::Map<String, Value> {
    result.structured_content.as_ref().unwrap()
}
fn code(result: &ToolResult) -> &str {
    assert_eq!(result.is_error, Some(true), "{}", text(result));
    structured(result)["code"].as_str().unwrap()
}
fn ok(result: &ToolResult) -> &str {
    assert_eq!(result.is_error, None, "{}", text(result));
    text(result)
}
fn utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

#[cfg(unix)]
#[tokio::test]
async fn coordinated_sessions_reject_stale_mutations() {
    use dim_agent_capabilities::workspace_tools::files::FileWriteCoordinator;
    use std::sync::Arc;
    let mut workspace = Workspace::new();
    let coordinator = Arc::new(FileWriteCoordinator::default());
    workspace.tools = FileTools::with_coordinator(
        workspace.root.clone(),
        workspace.root.clone(),
        workspace.root.join("outputs"),
        coordinator.clone(),
    );
    let other = FileTools::with_coordinator(
        workspace.root.clone(),
        workspace.root.clone(),
        workspace.root.join("outputs"),
        coordinator,
    );
    workspace.put("shared", b"before\r\n");
    let (a, b) = (branch(), branch());
    workspace.read(&a, "shared").await;
    let call = |name: &str, input: Value| ToolCall {
        id: "other".into(),
        name: name.into(),
        input: input.as_object().unwrap().clone(),
    };
    other
        .execute(
            &call("read", json!({"path": "shared"})),
            &b,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let second_call = call("write", json!({"path": "shared", "content": "two\n"}));
    let (first, second) = tokio::join!(
        workspace.call(&a, "write", json!({"path": "shared", "content": "one\n"})),
        other.execute(&second_call, &b, CancellationToken::new())
    );
    let second = second.unwrap();
    assert_ne!(first.is_error, second.is_error);
    let rejected = if first.is_error == Some(true) {
        &first
    } else {
        &second
    };
    assert_eq!(code(rejected), "file_modified_since_read");
    assert!(matches!(
        workspace.bytes("shared").as_slice(),
        b"one\r\n" | b"two\r\n"
    ));
    assert_eq!(fs::read_dir(&workspace.root).unwrap().count(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn replacement_preserves_permissions_and_open_original() {
    use std::{io::Read, os::unix::fs::PermissionsExt};
    let workspace = Workspace::new();
    workspace.put("private", b"before\r\n");
    fs::set_permissions(workspace.path("private"), fs::Permissions::from_mode(0o600)).unwrap();
    let mut original = fs::File::open(workspace.path("private")).unwrap();
    let reads = branch();
    workspace.read(&reads, "private").await;
    ok(&workspace
        .call(
            &reads,
            "write",
            json!({"path": "private", "content": "after\n"}),
        )
        .await);
    let mut old = Vec::new();
    original.read_to_end(&mut old).unwrap();
    assert_eq!(old, b"before\r\n");
    assert_eq!(workspace.bytes("private"), b"after\r\n");
    assert_eq!(
        fs::metadata(workspace.path("private"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(fs::read_dir(&workspace.root).unwrap().count(), 1);
}

#[test]
fn definitions_match_the_ts_tool_contracts() {
    let definitions = FileTools::definitions();
    let summary: Vec<(&str, Option<&str>)> = definitions
        .iter()
        .map(|definition| (definition.name.as_str(), definition.risk.as_deref()))
        .collect();
    assert_eq!(
        summary,
        [
            ("read", Some("read_filesystem")),
            ("write", Some("write_filesystem")),
            ("edit", Some("write_filesystem")),
            ("glob", Some("read_filesystem")),
            ("grep", Some("read_filesystem")),
        ]
    );
    assert!(
        definitions
            .iter()
            .all(|definition| FileTools::handles(&definition.name))
    );
    assert!(!FileTools::handles("exec"));
    assert_eq!(
        definitions[0].description,
        "Read a text file from the filesystem. Returns text with line numbers. Use offset and limit for large files."
    );
}

#[tokio::test]
async fn edits_and_writes_preserve_bom_and_line_endings() {
    let workspace = Workspace::new();
    let reads = branch();

    workspace.put("crlf.txt", b"one\r\ntwo\r\nthree\r\n");
    ok(&workspace.read(&reads, "crlf.txt").await);
    ok(&workspace.edit(&reads, "crlf.txt", "two", "2a\n2b").await);
    assert_eq!(workspace.bytes("crlf.txt"), b"one\r\n2a\r\n2b\r\nthree\r\n");
    ok(&workspace
        .call(
            &reads,
            "write",
            json!({ "path": "crlf.txt", "content": "x\ny\n" }),
        )
        .await);
    assert_eq!(workspace.bytes("crlf.txt"), b"x\r\ny\r\n");

    workspace.put("bom.txt", b"\xef\xbb\xbfalpha\nbeta\n");
    let read = workspace.read(&reads, "bom.txt").await;
    assert_eq!(ok(&read), "1\talpha\n2\tbeta");
    assert_eq!(structured(&read)["bom"], "utf8");
    ok(&workspace.edit(&reads, "bom.txt", "beta", "gamma").await);
    assert_eq!(workspace.bytes("bom.txt"), b"\xef\xbb\xbfalpha\ngamma\n");
    ok(&workspace
        .call(
            &reads,
            "write",
            json!({ "path": "bom.txt", "content": "delta\n" }),
        )
        .await);
    assert_eq!(workspace.bytes("bom.txt"), b"\xef\xbb\xbfdelta\n");

    let mut utf16 = vec![0xff, 0xfe];
    utf16.extend(utf16le("héllo\r\nwörld\r\n"));
    workspace.put("utf16.txt", &utf16);
    let read = workspace.read(&reads, "utf16.txt").await;
    assert_eq!(ok(&read), "1\théllo\n2\twörld");
    assert_eq!(structured(&read)["encoding"], json!({ "kind": "utf16le" }));
    ok(&workspace.edit(&reads, "utf16.txt", "wörld", "wörld!").await);
    let mut expected = vec![0xff, 0xfe];
    expected.extend(utf16le("héllo\r\nwörld!\r\n"));
    assert_eq!(workspace.bytes("utf16.txt"), expected);

    workspace.put("mixed.txt", b"a\r\nb\nc\r\nd\n");
    let read = workspace.read(&reads, "mixed.txt").await;
    assert_eq!(structured(&read)["eol"], "mixed");
    ok(&workspace.edit(&reads, "mixed.txt", "d", "d1\nd2").await);
    assert_eq!(workspace.bytes("mixed.txt"), b"a\r\nb\nc\r\nd1\nd2\n");
    ok(&workspace.edit(&reads, "mixed.txt", "c\nd1", "C\nD1").await);
    assert_eq!(workspace.bytes("mixed.txt"), b"a\r\nb\nC\r\nD1\nd2\n");
    ok(&workspace.edit(&reads, "mixed.txt", "a", "a1\na2").await);
    assert_eq!(
        workspace.bytes("mixed.txt"),
        b"a1\r\na2\r\nb\nC\r\nD1\nd2\n"
    );

    workspace.put("quotes.txt", "say “hi”  \nnext\n".as_bytes());
    ok(&workspace.read(&reads, "quotes.txt").await);
    let edited = workspace
        .edit(&reads, "quotes.txt", "say \"hi\"\nnext", "said hi\nnext")
        .await;
    ok(&edited);
    assert_eq!(workspace.bytes("quotes.txt"), b"said hi\nnext\n");
    assert!(
        structured(&edited)["diff"]
            .as_str()
            .unwrap()
            .contains("-say “hi”")
    );
}

#[tokio::test]
async fn writes_require_a_fresh_read_on_the_same_branch() {
    let workspace = Workspace::new();
    let main = branch();
    workspace.put("a.txt", b"original\n");

    let unread = workspace
        .call(&main, "write", json!({ "path": "a.txt", "content": "new" }))
        .await;
    assert_eq!(code(&unread), "text_binding_required");
    assert_eq!(
        code(&workspace.edit(&main, "a.txt", "original", "x").await),
        "text_binding_required"
    );

    ok(&workspace.read(&main, "a.txt").await);
    let child = branch();
    let foreign = workspace
        .call(
            &child,
            "write",
            json!({ "path": "a.txt", "content": "child" }),
        )
        .await;
    assert_eq!(code(&foreign), "text_binding_required");
    assert_eq!(
        code(&workspace.edit(&child, "a.txt", "original", "x").await),
        "text_binding_required"
    );

    workspace.put("a.txt", b"changed outside\n");
    let stale = workspace
        .call(
            &main,
            "write",
            json!({ "path": "./a.txt", "content": "new" }),
        )
        .await;
    assert_eq!(code(&stale), "file_modified_since_read");
    assert_eq!(
        code(&workspace.edit(&main, "a.txt", "changed", "x").await),
        "file_modified_since_read"
    );
    assert_eq!(workspace.bytes("a.txt"), b"changed outside\n");

    let created = workspace
        .call(
            &child,
            "write",
            json!({ "path": "sub/new.txt", "content": "one\n" }),
        )
        .await;
    assert_eq!(ok(&created), "Wrote sub/new.txt");
    ok(&workspace.edit(&child, "sub/new.txt", "one", "two").await);
    assert_eq!(workspace.bytes("sub/new.txt"), b"two\n");
}

/// One blocking thread makes the order of file work observable: a gate task
/// queued behind the path resolutions runs only after both finished, and the
/// holder's file work then waits behind the gate.
#[test]
fn edits_of_one_file_run_in_fifo_order_and_waiters_can_cancel() {
    use futures_util::FutureExt;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let workspace = Workspace::new();
        let reads = branch();
        workspace.put("pair.txt", b"left right\n");
        ok(&workspace.read(&reads, "pair.txt").await);
        let (first, second) = tokio::join!(
            workspace.edit(&reads, "pair.txt", "left", "L"),
            workspace.edit(&reads, "pair.txt", "right", "R"),
        );
        ok(&first);
        ok(&second);
        assert_eq!(workspace.bytes("pair.txt"), b"L R\n");

        let gate = |release: std::sync::mpsc::Receiver<()>| {
            let (started, running) = tokio::sync::oneshot::channel();
            tokio::task::spawn_blocking(move || {
                started.send(()).unwrap();
                release.recv().unwrap();
            });
            running
        };
        let (open_first, first_gate) = std::sync::mpsc::channel();
        gate(first_gate).await.unwrap();
        let waiter = CancellationToken::new();
        let mut holder = std::pin::pin!(workspace.edit(&reads, "pair.txt", "L", "HEAD"));
        let mut queued = std::pin::pin!(workspace.try_call(
            &reads,
            "edit",
            json!({ "path": "pair.txt", "oldText": "HEAD", "newText": "late" }),
            waiter.clone(),
        ));
        assert!(holder.as_mut().now_or_never().is_none());
        assert!(queued.as_mut().now_or_never().is_none());
        let (open_second, second_gate) = std::sync::mpsc::channel();
        let second = gate(second_gate);
        open_first.send(()).unwrap();
        second.await.unwrap();
        assert!(holder.as_mut().now_or_never().is_none());
        assert!(queued.as_mut().now_or_never().is_none());
        waiter.cancel();
        let queued = queued
            .now_or_never()
            .expect("a cancelled waiter returns at once");
        assert!(matches!(queued, Err(HostError::Cancelled(_))), "{queued:?}");
        open_second.send(()).unwrap();
        ok(&holder.await);
        assert_eq!(workspace.bytes("pair.txt"), b"HEAD R\n");

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let result = workspace
            .try_call(&reads, "read", json!({ "path": "pair.txt" }), cancelled)
            .await;
        assert!(matches!(result, Err(HostError::Cancelled(_))));
    });
}

#[tokio::test]
async fn read_rejects_non_text_targets() {
    let workspace = Workspace::new();
    let reads = branch();
    workspace.put("latin.txt", b"caf\xe9\n");
    workspace.put("blob.dat", b"ab\0cd");
    workspace.put("archive.zip", b"PK");
    fs::create_dir(workspace.path("dir")).unwrap();

    assert_eq!(
        code(&workspace.read(&reads, "latin.txt").await),
        "text_encoding_required"
    );
    assert_eq!(
        code(&workspace.read(&reads, "blob.dat").await),
        "binary_unsupported"
    );
    assert_eq!(
        code(&workspace.read(&reads, "archive.zip").await),
        "binary_unsupported"
    );
    let directory = workspace.read(&reads, "dir").await;
    assert_eq!(code(&directory), "is_directory");
    assert_eq!(text(&directory), "Cannot read directory: dir");
    let missing = workspace.read(&reads, "Latin.TXT.missing").await;
    assert_eq!(code(&missing), "file_not_found");
    assert_eq!(
        code(&workspace.call(&reads, "read", json!({ "path": "" })).await),
        "invalid_input"
    );
}

#[tokio::test]
async fn read_pages_long_files() {
    let workspace = Workspace::new();
    let reads = branch();
    workspace.put("lines.txt", b"a\nb\nc\nd\n");
    let page = workspace
        .call(
            &reads,
            "read",
            json!({ "path": "lines.txt", "offset": 2, "limit": 2 }),
        )
        .await;
    assert_eq!(
        ok(&page),
        "2\tb\n3\tc\n\nFile is larger than the safe read budget for one response. Showing lines 2-3 of 4. Continue with read({ path: \"lines.txt\", offset: 4 })."
    );
    let beyond = workspace
        .call(&reads, "read", json!({ "path": "lines.txt", "offset": 9 }))
        .await;
    assert_eq!(code(&beyond), "offset_out_of_range");
    assert_eq!(text(&beyond), "Read offset 9 exceeds file length.");
}

#[tokio::test]
async fn read_offloads_large_results_to_the_output_directory() {
    let workspace = Workspace::new();
    let reads = branch();
    let line = "x".repeat(99);
    let body: String = (0..400).map(|_| format!("{line}\n")).collect();
    workspace.put("big.txt", body.as_bytes());
    let result = workspace.read(&reads, "big.txt").await;
    let content = ok(&result);
    assert_eq!(structured(&result)["offloaded"], true);
    let output = structured(&result)["outputPath"].as_str().unwrap();
    assert!(
        output.starts_with(
            workspace
                .root
                .parent()
                .unwrap()
                .join("outputs")
                .to_str()
                .unwrap()
        )
    );
    let full = fs::read_to_string(output).unwrap();
    assert_eq!(full.lines().count(), 400);
    assert!(full.starts_with(&format!("1\t{line}\n2\t")));
    let (head, preview) = content
        .split_once("\n\nPreview (first 2000 chars):\n")
        .unwrap();
    assert_eq!(
        head,
        format!(
            "Read output too large ({} bytes). Full output: {output}",
            full.len()
        )
    );
    assert_eq!(preview, &full[..2000]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(Path::new(output)), 0o600);
        assert_eq!(mode(Path::new(output).parent().unwrap()), 0o700);
    }
    let last = format!("{line}\n");
    let edit = workspace
        .call(
            &reads,
            "edit",
            json!({ "path": "big.txt", "oldText": last, "newText": "y\n", "replaceAll": true }),
        )
        .await;
    ok(&edit);
    assert_eq!(workspace.bytes("big.txt"), "y\n".repeat(400).into_bytes());

    workspace.put("small.txt", b"hi\n");
    let small = workspace.read(&reads, "small.txt").await;
    assert_eq!(ok(&small), "1\thi");
    assert!(!structured(&small).contains_key("offloaded"));
}

#[tokio::test]
async fn oversized_lines_report_partial_content_without_skipping_the_tail() {
    let workspace = Workspace::new();
    let reads = branch();
    for (body, partial) in [
        (format!("{}TAIL", "a".repeat(262_138)), false),
        (format!("{}TAIL", "a".repeat(300_000)), true),
        (format!("{}TAIL\r\nnext", "😀".repeat(150_000)), true),
    ] {
        workspace.put("long.txt", body.as_bytes());
        let result = workspace.read(&reads, "long.txt").await;
        ok(&result);
        let full = fs::read_to_string(structured(&result)["outputPath"].as_str().unwrap()).unwrap();
        if partial {
            assert!(text(&result).contains("truncated"));
            assert!(full.contains("truncated"));
            assert!(!full.contains("TAIL"));
            assert!(!full.contains("offset: 2"));
        } else {
            assert!(full.ends_with("TAIL"));
            assert!(!full.contains("truncated"));
        }
        workspace.put("long.txt", body.replace("TAIL", "CHANGED").as_bytes());
        let edit = workspace.edit(&reads, "long.txt", "a", "b").await;
        assert_eq!(code(&edit), "file_modified_since_read");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn special_files_are_refused_without_blocking() {
    let workspace = Workspace::new();
    let reads = branch();
    let fifo = workspace.path("pipe");
    let c_path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: plain syscall creating a FIFO in the test directory.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        workspace.read(&reads, "pipe"),
    )
    .await
    .expect("read of a FIFO returns");
    assert_eq!(code(&result), "not_regular_file");
    let socket_path = std::env::temp_dir().join(uuid::Uuid::new_v4().simple().to_string());
    let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    let socket = workspace.read(&reads, socket_path.to_str().unwrap()).await;
    drop(listener);
    fs::remove_file(socket_path).unwrap();
    assert_eq!(code(&socket), "read_error");
}

#[tokio::test]
async fn read_windows_far_into_a_file_and_rejects_overflowing_offsets() {
    let workspace = Workspace::new();
    let reads = branch();
    let body: String = (1..=5000).map(|n| format!("line {n}\n")).collect();
    workspace.put("log.txt", body.as_bytes());
    let page = workspace
        .call(
            &reads,
            "read",
            json!({ "path": "log.txt", "offset": 4999, "limit": 5 }),
        )
        .await;
    assert_eq!(ok(&page), "4999\tline 4999\n5000\tline 5000");
    let beyond = workspace
        .call(
            &reads,
            "read",
            json!({ "path": "log.txt", "offset": u64::MAX as f64, "limit": 2 }),
        )
        .await;
    assert_eq!(code(&beyond), "offset_out_of_range");
    let mut broken = body.into_bytes();
    broken.push(0xFF);
    workspace.put("broken.txt", &broken);
    let beyond = workspace
        .call(
            &reads,
            "read",
            json!({ "path": "broken.txt", "offset": u64::MAX as f64, "limit": 2 }),
        )
        .await;
    assert_eq!(code(&beyond), "offset_out_of_range");
}

#[tokio::test]
async fn a_failed_offload_does_not_bind_the_file() {
    let workspace = Workspace::new();
    let reads = branch();
    workspace.put("blocker", b"");
    let tools = FileTools::new(
        workspace.root.clone(),
        workspace.root.clone(),
        workspace.path("blocker").join("outputs"),
    );
    let body: String = (0..400).map(|_| format!("{}\n", "x".repeat(99))).collect();
    workspace.put("big.txt", body.as_bytes());
    let call = |name: &str, input: Value| ToolCall {
        id: "call".into(),
        name: name.into(),
        input: input.as_object().unwrap().clone(),
    };
    let read = tools
        .execute(
            &call("read", json!({ "path": "big.txt" })),
            &reads,
            CancellationToken::new(),
        )
        .await;
    assert!(read.is_err(), "the offload failed");
    let write = tools
        .execute(
            &call("write", json!({ "path": "big.txt", "content": "replaced" })),
            &reads,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(code(&write), "text_binding_required");
    assert_eq!(workspace.bytes("big.txt"), body.into_bytes());
}

#[tokio::test]
async fn edit_inserts_new_text_literally() {
    let workspace = Workspace::new();
    let reads = branch();
    workspace.put("run.sh", b"echo PID\n");
    ok(&workspace.read(&reads, "run.sh").await);
    ok(&workspace.edit(&reads, "run.sh", "PID", "$$ $& $' $`").await);
    assert_eq!(workspace.bytes("run.sh"), b"echo $$ $& $' $`\n");
}

#[tokio::test]
async fn legacy_encodings_round_trip_and_reject_unrepresentable_text() {
    let workspace = Workspace::new();
    let reads = branch();
    let (gbk, _, _) = encoding_rs::GBK.encode("中文内容\r\n第二行\r\n");
    workspace.put("gbk.txt", &gbk);

    assert_eq!(
        code(&workspace.read(&reads, "gbk.txt").await),
        "text_encoding_required"
    );
    let read = workspace
        .call(
            &reads,
            "read",
            json!({ "path": "gbk.txt", "encoding": "gbk" }),
        )
        .await;
    assert_eq!(ok(&read), "1\t中文内容\n2\t第二行");
    assert_eq!(
        structured(&read)["encoding"],
        json!({ "kind": "legacy", "name": "gbk" })
    );
    assert_eq!(structured(&read)["encodingSource"], "user");

    ok(&workspace.edit(&reads, "gbk.txt", "第二行", "第三行").await);
    let (expected, _, _) = encoding_rs::GBK.encode("中文内容\r\n第三行\r\n");
    assert_eq!(workspace.bytes("gbk.txt"), expected.as_ref());

    let before = workspace.bytes("gbk.txt");
    let emoji = workspace.edit(&reads, "gbk.txt", "第三行", "😀").await;
    assert_eq!(code(&emoji), "text_not_representable");
    let overwrite = workspace
        .call(
            &reads,
            "write",
            json!({ "path": "gbk.txt", "content": "😀" }),
        )
        .await;
    assert_eq!(code(&overwrite), "text_not_representable");
    assert_eq!(workspace.bytes("gbk.txt"), before);

    let cp936 = workspace
        .call(
            &reads,
            "read",
            json!({ "path": "gbk.txt", "encoding": "cp936" }),
        )
        .await;
    assert_eq!(ok(&cp936), "1\t中文内容\n2\t第三行");
    let conflict = {
        workspace.put("bom.txt", b"\xef\xbb\xbfx");
        workspace
            .call(
                &reads,
                "read",
                json!({ "path": "bom.txt", "encoding": "gbk" }),
            )
            .await
    };
    assert_eq!(code(&conflict), "text_encoding_conflict");
}

/// Mirrors `executor.acceptance.test.ts` (ripgrep runner expectations).
fn search_fixture(workspace: &Workspace) {
    fs::create_dir_all(workspace.path(".git")).unwrap();
    workspace.put(".ignore", b"ignored.ts\n");
    workspace.put(".hidden.ts", b"needle hidden\n");
    workspace.put("ignored.ts", b"needle ignored\n");
    let visible: Vec<String> = (0..7)
        .map(|index| format!("needle visible {index}"))
        .collect();
    workspace.put("visible.ts", visible.join("\n").as_bytes());
    workspace.put("bundle.min.js", b"needle minified\n");
    workspace.put("node_modules/excluded.ts", b"needle excluded\n");
    fs::File::create(workspace.path("large.bin"))
        .unwrap()
        .set_len(10 * 1024 * 1024 + 1)
        .unwrap();
}

fn glob_paths(result: &ToolResult) -> Vec<String> {
    let mut paths: Vec<String> = structured(result)["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap().to_owned())
        .collect();
    paths.sort();
    paths
}

#[tokio::test]
async fn glob_matches_the_ripgrep_runner() {
    let workspace = Workspace::new();
    let reads = branch();
    search_fixture(&workspace);

    let all = workspace
        .call(&reads, "glob", json!({ "pattern": "*.ts" }))
        .await;
    assert_eq!(glob_paths(&all), [".hidden.ts", "ignored.ts", "visible.ts"]);
    assert_eq!(structured(&all)["truncated"], false);
    assert!(ok(&all).starts_with("Matched 3 files. Showing 3 files. Not truncated.\n\n"));
    let first = &structured(&all)["files"][0];
    assert_eq!(first["label"], first["path"]);
    assert_eq!(
        Path::new(first["fsPath"].as_str().unwrap()),
        workspace.path(first["path"].as_str().unwrap())
    );

    for name in [
        "src/a.ts",
        "src/]file.ts",
        "src/nested/b.ts",
        "src/nested/afile.ts",
        "src/nested/c.ts",
    ] {
        workspace.put(name, b"");
    }
    let scoped = workspace
        .call(&reads, "glob", json!({ "pattern": "src/**/[ab].ts" }))
        .await;
    assert_eq!(glob_paths(&scoped), ["src/a.ts", "src/nested/b.ts"]);
    let bracket = workspace
        .call(&reads, "glob", json!({ "pattern": "src/**/[]a]file.ts" }))
        .await;
    assert_eq!(
        glob_paths(&bracket),
        ["src/]file.ts", "src/nested/afile.ts"]
    );

    let limited = workspace
        .call(&reads, "glob", json!({ "pattern": "**/*.TS", "limit": 2 }))
        .await;
    assert_eq!(structured(&limited)["truncated"], true);
    assert!(ok(&limited).starts_with(
        "Matched at least 2 files. Showing 2 files. More files may remain; narrow path or pattern and retry.\n\n"
    ));

    let file = workspace
        .call(
            &reads,
            "glob",
            json!({ "pattern": "*.ts", "path": "src/a.ts" }),
        )
        .await;
    assert_eq!(glob_paths(&file), ["a.ts"]);

    for pattern in [
        "*",
        "a*",
        "**",
        "/abs/*.ts",
        "a/../b.ts",
        "*.{ts,js}",
        "!x.ts",
    ] {
        let rejected = workspace
            .call(&reads, "glob", json!({ "pattern": pattern }))
            .await;
        assert_eq!(code(&rejected), "invalid_input", "{pattern}");
        assert_eq!(
            text(&rejected),
            "pattern must be a relative git-style glob with at least two consecutive literal characters in its final component, for example *.mjs or **/*.rs; bare *, **, **/*, leading ./, absolute paths, .., braces, and backslashes are not supported. Use path to choose the search directory"
        );
    }
    let home = FileTools::new(
        workspace.root.clone(),
        workspace.root.clone(),
        workspace.root.join("outputs"),
    );
    let call = ToolCall {
        id: "call".into(),
        name: "glob".into(),
        input: json!({ "pattern": "*.ts" }).as_object().unwrap().clone(),
    };
    let open_ended = home
        .execute(&call, &reads, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        text(&open_ended),
        "Open-ended searches cannot use the filesystem root or user home"
    );
}

#[tokio::test]
async fn grep_survives_byte_matches_and_rejects_bad_include_globs() {
    let workspace = Workspace::new();
    let reads = branch();
    workspace.put("accent.txt", "café\n".as_bytes());
    let result = workspace
        .call(
            &reads,
            "grep",
            json!({ "pattern": "(?-u:\\xA9)", "path": "accent.txt", "outputMode": "content" }),
        )
        .await;
    assert_eq!(result.is_error, None, "{}", text(&result));
    assert_eq!(structured(&result)["matches"][0]["column"], 4);
    for pattern in ["(?-u:\\xA9)", "(?-u:\\xC3)"] {
        let result = workspace
            .call(
                &reads,
                "grep",
                json!({ "pattern": pattern, "path": "accent.txt", "outputMode": "content", "multiline": true }),
            )
            .await;
        assert_eq!(result.is_error, None, "{}", text(&result));
        assert_eq!(structured(&result)["matches"][0]["column"], 4);
        assert_eq!(structured(&result)["matches"][0]["content"], "é");
    }
    for path in ["accent.txt", "."] {
        let rejected = workspace
            .call(
                &reads,
                "grep",
                json!({ "pattern": "caf", "path": path, "include": "[" }),
            )
            .await;
        assert_eq!(code(&rejected), "invalid_input", "path: {path}");
        assert!(
            text(&rejected).contains("Invalid glob"),
            "{}",
            text(&rejected)
        );
    }
}

#[tokio::test]
async fn grep_matches_the_ripgrep_runner() {
    let workspace = Workspace::new();
    let reads = branch();
    search_fixture(&workspace);

    let found = workspace
        .call(
            &reads,
            "grep",
            json!({ "pattern": "needle", "literal": true, "lineNumbers": true }),
        )
        .await;
    let matches = structured(&found)["matches"].as_array().unwrap();
    let mut files: Vec<&str> = matches
        .iter()
        .map(|row| row["file"].as_str().unwrap())
        .collect();
    files.dedup();
    files.sort();
    assert_eq!(files, [".hidden.ts", "visible.ts"]);
    assert_eq!(structured(&found)["truncated"], true);
    assert_eq!(
        matches
            .iter()
            .filter(|row| row["file"] == "visible.ts")
            .count(),
        5
    );
    assert_eq!(
        ok(&found),
        "Matched at least 6 records in 2 shown files. More matches may remain; narrow path or include and retry.\n\n\
         .hidden.ts\n  1| needle hidden\n\
         visible.ts\n  1| needle visible 0\n  2| needle visible 1\n  3| needle visible 2\n  4| needle visible 3\n  5| needle visible 4"
    );

    let large = workspace
        .call(
            &reads,
            "grep",
            json!({ "pattern": "needle", "path": "large.bin" }),
        )
        .await;
    assert_eq!(
        structured(&large)["skippedFiles"],
        json!([{ "file": "large.bin", "reason": "too_large" }])
    );

    workspace.put(
        "ctx/a.txt",
        b"zero\nOne target\ntwo\nthree\nfour target\nfive\n",
    );
    let context = workspace
        .call(
            &reads,
            "grep",
            json!({ "pattern": "target", "path": "ctx", "-C": 1, "-n": true }),
        )
        .await;
    assert_eq!(
        ok(&context),
        "Matched 2 records in 1 shown files. Not truncated.\n\n\
         a.txt\n  1| zero\n  2| One target\n  3| two\n  4| three\n  5| four target\n  6| five"
    );
    assert_eq!(structured(&context)["matches"][0]["column"], 5);

    let files_mode = workspace
        .call(
            &reads,
            "grep",
            json!({ "pattern": "Target", "include": "*.{txt,md}", "outputMode": "files" }),
        )
        .await;
    assert_eq!(
        ok(&files_mode),
        "Matched 0 files in 0 shown files. Not truncated."
    );
    let files_mode = workspace
        .call(
            &reads,
            "grep",
            json!({ "pattern": "target", "include": "*.{txt,md}", "output_mode": "files" }),
        )
        .await;
    assert_eq!(
        ok(&files_mode),
        "Matched 1 files in 1 shown files. Not truncated.\n\nctx/a.txt: 2 matches"
    );

    let multiline = workspace
        .call(
            &reads,
            "grep",
            json!({ "pattern": "two\\nthree", "path": "ctx", "multiline": true }),
        )
        .await;
    assert_eq!(structured(&multiline)["matches"][0]["line"], 3);
    assert_eq!(
        structured(&multiline)["matches"][0]["content"],
        "two\nthree"
    );

    for (input, message) in [
        (
            json!({ "pattern": "(a+)+" }),
            "Potentially catastrophic quantifier pattern detected (group with alternation/wildcard + broad quantifier)",
        ),
        (
            json!({ "pattern": "(x)\\1" }),
            "Backreferences (\\1, \\k<name>) are not allowed",
        ),
        (
            json!({ "pattern": "x", "ignoreCase": true }),
            "ignoreCase is no longer supported",
        ),
        (
            json!({ "pattern": "x", "limit": 101 }),
            "limit must be an integer from 1 to 100",
        ),
        (
            json!({ "pattern": "x", "-A": 11 }),
            "afterContext must be an integer from 0 to 10",
        ),
    ] {
        let rejected = workspace.call(&reads, "grep", input).await;
        assert_eq!(code(&rejected), "invalid_input");
        assert_eq!(text(&rejected), message);
    }
}
