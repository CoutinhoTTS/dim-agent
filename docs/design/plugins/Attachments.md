# Attachments

Status: Draft. Overall design: [overview §5.4](../overview.md#54-attachments). Phase: P1. Language: Rust.

Turns attachments into final content at the submission boundary. Images become image blocks in the submitted `user` content; text attachments (large pastes) are stored as files on the Session's resource node through the Session-bound FileStorage and referenced by path, for the model to `read` as needed. Attachment files follow the Session: a fork keeps the source's attachments available to the new Session, and they are removed only when no Session referencing them remains. Process plugin, no events.

## 1. User flows

1. The user pastes a screenshot (`Ctrl+V`) and types a question. The CLI reads the image bytes (clipboard or path) and submits `session.submit` with `user.content = [image(base64), text]`. Rpc runs the submission filters; Attachments sniffs the format (PNG), checks size and model support, and passes the final image block on. The input is written with the image as user content; the CLI deletes its temporary clipboard file.
2. The user pastes 3,000 lines of log. The CLI calls `attachment.put {sessionId, name: "paste-1.txt", text}` and gets a path on the Session's resource node; on submit it inserts a reference line containing that path into the user text. The model reads the file with `read` when it needs it.
3. The current model has no image input. Submitting an image is rejected at the boundary with `attachment_unsupported_model`; nothing is written; the CLI keeps the draft.
4. The user forks the Session. The fork's history still contains the source's attachment paths; they remain readable from the fork. Deleting the source Session later does not remove files the fork references.
5. The user deletes a Session; its attachment directory references are dropped, and files no longer referenced by any Session are removed.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Image validation and normalization into final `ContentBlock::Image` at the submit boundary | Reading the clipboard, paths, drag-and-drop, temporary terminal caches (CLI, overview §5.4) |
| Storing text attachments on the Session's resource node, returning their path | Formatting the reference line inserted into the user text (CLI, A6) |
| Attachment ownership across fork and delete | Copying the Timeline, Session metadata, fork/delete orchestration (SessionStore, Agents) |
| Rejecting images for models without image input | Model capability data (Provider catalog `vision`/`modalities`) |

## 3. Scope and dependencies

| Part | Scope | Injects | Provides |
| --- | --- | --- | --- |
| `AttachmentsPlugin` | Process (App Server) | `Arc<SessionResources>` | `SubmissionFilter` under name `attachments`; `SessionClosure` under name `attachments`; the `attachment.put` Session method handler for Rpc |

**Contracts depended on**

| Contract | Owner | Status | Use |
| --- | --- | --- | --- |
| `SubmissionFilter` / `SubmissionFiltering::filter(SubmissionContext, SubmissionDraft, CancellationToken) -> Result<SubmissionDraft, SubmissionRejected>` | Rpc (consumer-defined extension point) | Proposed Rpc supplement, specified in [Hooks §6.5](Hooks.md#65-submission-boundary-implemented-defined-by-rpc); not yet in overview §7.2 (§14) | Image boundary. Uses `SubmissionContext.session_id`, `resource_node`, `model_accepts_images` |
| `SessionResources::resolve(session_id) -> SessionResourceBinding { node, cwd, file_storage, host_platform, isolation }` | Agents | Proposed in [ShellInput §3](ShellInput.md#3-scope-and-dependencies) (overview §7.1 supplement, #5) | Session-bound FileStorage on the resource node |
| `FileStorage::register_root`, `path`, `lock`, `replace` | FileStorage | Existing | Atomic file creation under an authorized root |
| `FileStorage` directory listing and removal | FileStorage | Supplement required (§14) | Ownership markers, garbage removal |
| `SessionClosure { fork(source, target, node), delete(session, node) }` | SessionStore (consumer-defined: fork copies "facts and their reference closure", overview §7.2) | Proposed here for the SessionStore supplement (§14) | Fork and delete participation |
| Session method registration in Rpc | Rpc | Existing pattern (overview §9 Protocol) | `attachment.put` |

Attachments never reads Session or branch state: the resource node and model capability arrive in `SubmissionContext` (filled by Rpc), the node binding from `SessionResources`. It does not know Hooks, SessionStore internals or Provider: the extension points are defined by their consumers.

**Contracts defined here**: the `attachment.put` method, the on-disk layout of the attachment root (§5), the error codes, the accepted image formats and limits.

## 4. Events

None (overview §5.4). Images are part of the existing `input/message` (`user.content`), written by Rpc's `SessionAccess::relay` after the filters (the user stays the author, [session-access](../contracts/session-access.md) §6 "Relay"). Text attachments appear only as paths inside user text. Ownership changes follow Session lifecycle operations that are process-level, not branch facts; nothing later in a branch depends on them.

## 5. State

All state lives on the resource node, under one authorized FileStorage root `attachments/` inside the node's dim home (A1):

```
attachments/
  files/<attachment_id>/<name>          immutable content, written once
  files/<attachment_id>/owners/<session_id>   empty marker: this Session references the file
  sessions/<session_id>/<attachment_id>       empty marker: index for fork/delete
```

| State | Rebuild |
| --- | --- |
| Files and markers | On disk; read on demand; no memory cache across calls |
| Per-attachment lock | `FileStorage::lock` on `files/<attachment_id>`; serializes marker changes and removal |

`attachment_id` is a random UUID; the path never contains the Session id, so a fork's inherited history keeps valid paths (§7).

## 6. Interfaces

### 6.1 Submission filter (images)

Registered before Hooks so later filters see final content (A2; Hooks A18).

| Input block | Handling |
| --- | --- |
| `Text` | Unchanged |
| `Image { source: Base64 }` | Decode; sniff the format from magic bytes (the declared `media_type` is ignored); accept PNG, JPEG, WebP, GIF (feature list); set `media_type` to the sniffed type; enforce limits (A3); emit `Image { source: Base64, media_type, data }` |
| `Image { source: Url }` | Rejected `attachment_invalid` (A4) |
| Any image and `model_accepts_images == Some(false)` | Rejected `attachment_unsupported_model` |
| Any image and `model_accepts_images == None` | Accepted; the model adapter decides (A5) |

Limits (A3): at most 20 images per submission, 5 MiB decoded per image, 8000 px per side; violations are rejected, never silently resized. Blocks are returned in their original order. The filter writes nothing and has no side effect; cancellation simply drops the work.

Rejections: `SubmissionRejected { filter: "attachments", reason }`, surfaced by Rpc as `invalid_params` with `appError` `attachment_invalid` / `attachment_too_large` / `attachment_unsupported_model`.

### 6.2 Session method `attachment.put`

`attachment.put {sessionId, name, text}` → `{attachmentId, path, bytes, lines}`.

| Rule | Content |
| --- | --- |
| `name` | `[A-Za-z0-9._-]{1,64}`, no leading dot; defaults to `paste.txt` when absent |
| `text` | UTF-8 (JSON string), ≤ 10 MiB (A3); empty rejected |
| Node | `SessionResources::resolve(sessionId)`; written through that node's FileStorage |
| Write order | Create `files/<id>/<name>` atomically (`lock` + `replace`), then `owners/<sessionId>`, then `sessions/<sessionId>/<id>`; the path is returned only after all three exist |
| `path` | Absolute path on the resource node, as the model's `read` on that node resolves it |
| Errors | `session_closed`, `node_unavailable`, `attachment_invalid`, `attachment_too_large`, `service_reloading` |

`attachment.put` is a Session method (it acts on one Session's data), so it is in Rpc, available wherever Session access is (overview §9). Rpc acquires the Session as for other Session methods; Attachments does not hold a lease.

### 6.3 Session closure

```rust
impl SessionClosureParticipant for Attachments {
    /// Called by SessionStore's fork before the target Session is published.
    async fn fork(&self, source: &str, target: &str, node: Option<&str>) -> Result<(), ClosureError>;
    /// Called by SessionStore's delete after the Session is closed.
    async fn delete(&self, session: &str, node: Option<&str>) -> Result<(), ClosureError>;
}
```

| Operation | Steps |
| --- | --- |
| `fork` | For each marker in `sessions/<source>/`: under the attachment lock, create `files/<id>/owners/<target>` and `sessions/<target>/<id>`. No content copy: content is immutable and paths stay valid for the fork's inherited history (A7). Idempotent |
| `delete` | For each marker in `sessions/<session>/`: under the lock, remove `owners/<session>`; if `owners/` is empty, remove `files/<id>`; then remove the session marker; finally remove `sessions/<session>/`. Idempotent; a crash leaves only markers that a repeated `delete` finishes |

### 6.4 CLI entry (contract for the CLI layer)

The CLI reads images (clipboard, `@`, pasted paths) and sends them as base64 image blocks in `session.submit`; large pastes (CLI threshold, A6) go through `attachment.put` at submission, and the CLI inserts `[Pasted text: <path> (<lines> lines)]` (A6) into the user text. On rejection the draft is kept and the error shown. Temporary image files created by the CLI are deleted by the CLI after the submit reply.

## 7. Timeline and capability mechanics

| Step | Mechanics |
| --- | --- |
| Submit | Rpc `session.submit` → filters in registration order (Attachments, then Hooks, ...) → `SessionLease::relay(branch, RelayInput { user, mode, submission_id })` → `Session::input` → `Branch::input` → boundary `validate` → `Timeline::append`. Attachments only transforms the draft; the write is Rpc's |
| Consumption | InputQueue (`input/applied`) and Loop (`input/consumed`) per mode; existing |
| StateCell | None owned; the input lives in InputQueue's `StateCell<InputSnapshot>` until consumed |
| Recheck | None: no Timeline decision. `model_accepts_images` is the caller's value at submit time; a later model switch is not rechecked (§8) |
| Terminal facts | None; `attachment.put` has a committed file or an error |
| Reload | Filter reload: Rpc reloads its filter chain; submits in the window get `service_reloading`, nothing written. In-flight `attachment.put`: FileStorage cancellation is checked before commit; a file without markers is garbage that no Session references (A8) |
| Restart | No in-memory state; files and markers persist on the node |

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Image submitted while a turn runs | Input by mode (Queue/Steer) | Applied and consumed like any input | InputQueue, Loop |
| Turn `stop/interrupt` before the image input is consumed | applied < I | Pending; consumed by the next turn | Loop |
| Branch close between filter and write | close < write | `relay` fails `BranchMissing`; nothing written | Runtime facade |
| Model switched after submit to one without images | input < `context/configured` change < consumed | Image reaches the new model; its adapter rejects or drops it (existing behavior) | ModelProvider |
| Concurrent Steer/Queue/Append from other writers | Timeline order | Unaffected | InputQueue |
| `attachment.put`, then the submit is never sent | — | File owned by the Session; removed with it | Attachments |
| `attachment.put` racing Session delete | marker created after delete scanned | Marker survives; repeated cleanup or the next delete pass removes it (A8) | Attachments |
| Fork racing delete of the source | under per-attachment lock | Either the target marker exists before the owner check (file kept) or delete removed the file first and fork skips a missing file (fork reports it) | Attachments lock |
| Compaction | consumed < compaction | Image survives only through the summary; text attachment path survives if the summary keeps it; the file remains on disk | ContextPreparation |
| Child branch cut | consumed < cut | Child inherits the image block and path; same Session, same files | History projection |
| Capability or Rpc reload during submit | — | `service_reloading`; nothing written; client retries with the same `submissionId` after `session.input_lookup` | Rpc, client |
| Process restart | — | No state lost; paths valid | — |
| Resource node offline | — | Image filtering does not need the node (images are content); `attachment.put` fails `node_unavailable`; the model's later `read` fails like any `read` | SessionResources, WorkspaceTools |
| Duplicate or late `attachment.put` | — | Each call creates a new attachment; no deduplication (A9) | Attachments |
| Session unload/restore | — | Files independent of loading | — |
| Client detach | — | No effect | — |

## 9. Context mapping

| Attachment | What the model sees | Placement |
| --- | --- | --- |
| Image | `ContentBlock::Image` inside the user message of the input (user content after the input's raw blocks) | Consumed position (`input/consumed`) |
| Text attachment | The path inside the user text; the content only through a later `read` tool result | User message at the consumed position; content at that tool result's position |

Compaction keeps neither images nor file contents verbatim; only the summary. Child branches inherit consumed inputs before the cut; forked Sessions inherit the fixed prefix and the files stay readable (§6.3). Non-interactive Sessions (`exec` with images) use the same boundary when their first input comes through `session.submit`; inputs written by other process plugins through `SessionAccess::input` do not pass the filter (A10). The model never sees attachment ids, ownership markers, rejected submissions or diagnostics.

## 10. Errors and recovery

| Code | Meaning |
| --- | --- |
| `attachment_invalid` | Unsupported or undecodable image, URL source, bad name, empty text |
| `attachment_too_large` | Over a limit of §6.1/§6.2 |
| `attachment_unsupported_model` | Images for a model declared without image input |
| `node_unavailable` | Resource node services cannot be resolved |
| `session_closed`, `service_reloading` | As for other Session methods |

A failed filter leaves no fact. A failed `attachment.put` may leave an unreferenced file; `delete` of the Session and a startup sweep of `files/` without `owners/` entries remove it (A8). No mutation is retried automatically (FileStorage rule).

## 11. Security

Images are decoded only to validate headers and limits; content is not executed. Text attachments are written under an authorized FileStorage root with symlink-safe traversal and never at client-chosen paths (`name` is a file name, not a path). Files are readable by the model's `read` under the Session's permission policy; attachments confer no extra authority. Attachment contents and images are user data replayed to the model; the CLI must not attach secrets implicitly. Fork never widens access: the fork can read only files its inherited history references.

## 12. Acceptance

1. PNG/JPEG/WebP/GIF accepted with sniffed media types; a mislabeled JPEG is corrected; BMP and URL sources rejected; nothing written on rejection.
2. Non-vision model: `attachment_unsupported_model`; unknown capability: accepted.
3. Limits: 21 images, 6 MiB image, 9000 px side rejected.
4. `attachment.put` returns a path that `read` in the same Session resolves on the resource node (local and imported node).
5. Fork then delete source: fork still reads the file; delete fork: file removed.
6. Filter order: Hooks sees the normalized image block.
7. Interrupt, reload and branch-close rows of §8 through public RPC.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Attachment root is `attachments/` in the node's dim home | Judgment | Another location; layout unchanged |
| A2 | Attachments filters before Hooks | Judgment; Hooks A18 | Hooks sees raw client blocks |
| A3 | Limits: 20 images, 5 MiB, 8000 px, 10 MiB text; reject rather than resize | Edge-case reference; judgment | Different limits or server-side resizing |
| A4 | URL image sources are rejected | Overview §5.4 "final content"; judgment | The model provider fetches URLs |
| A5 | Unknown model capability accepts images | Judgment | Unknown is rejected |
| A6 | CLI chooses the paste threshold and reference format | Overview §8 (CLI); judgment | Server returns a formatted reference |
| A7 | Fork shares immutable files by ownership markers instead of copying bytes | Overview §5.4/§7.2 ("copied with the Session", "historical references intact"); judgment: copied bytes at a new path would break inherited paths | Byte copy plus path rewriting, impossible for fixed history |
| A8 | Unreferenced files are removed by delete passes and a startup sweep | Judgment | Orphans accumulate |
| A9 | No deduplication of identical text attachments | Judgment | Content-addressed ids |
| A10 | Only `session.submit` passes the filter | Hooks §6.5; judgment | Other writers validate their own images |
| A11 | A fork keeps the source's resource node | Overview §3.1 (node fixed at creation); judgment for forks | Fork to another node must copy files to the same path there |

## 14. Open questions

| Question | Description |
| --- | --- |
| Overview §5.4 "copied" | Implemented as shared ownership (A7); the overview wording should read "kept available to the fork" |
| `SubmissionFilter` | Rpc supplement not yet registered in overview §7.2 |
| `SessionClosure` | SessionStore supplement (overview §7.2) must define this participant contract and call it on fork and delete |
| FileStorage listing/removal | Supplement needed: list a directory and remove an empty directory or file under a root |
| `SessionResources` | Shared with ShellInput and Hooks; not yet in overview §7.1 |
