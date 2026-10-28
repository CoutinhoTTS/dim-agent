# Recoverable conversation import

Agents and SessionStore own this generic protocol v3 boundary for final typed
inputs and file bytes. [Migration](../plugins/Migration.md) converts legacy data
before calling it. Native storage and a protected local connection are required;
WebSocket and earlier protocol connections cannot call it.

| Method | Request | Result |
| --- | --- | --- |
| `session.import.begin` | `key`, `fingerprint`, absolute `cwd`, optional `model`, `policy`, `title`, expected `chunks` and `files` | conversation receipt |
| `session.import.chunk` | `key`, zero-based `index`, final `input` | receipt with next cursor |
| `session.import.file` | `key`, source file `id`, safe `name`, `sha256`, total `bytes`, `offset`, byte-array `content` | completed file receipt, or null while incomplete |
| `session.import.finish` | `key` | final conversation receipt |

The receipt has `agentId`, `sessionId`, `status` (`preparing`, `committed`,
`deleted`, `conflict`), `nextChunk` and completed file receipts (`id`, `fileId`, `path`, `bytes`).
Keys are nonempty caller-owned identities of at most 512 bytes; fingerprints are
64-character SHA-256 digests covering the source object. Reusing a key with
different source content or request returns `conflict` with the existing identity and changes no data. A deleted target returns `deleted` even if the source changed. Identity never derives from path.
The target default Agent owns the reservation and caps its policy as for launch.
Explicit model references are stored without resolution or availability checks.
Historical cwd need only be absolute; normal client use determines usability.

Begin reserves an unpublished Session and main branch. No protocol owner or
ordinary listing, attach or continuation is available yet. Repeating begin
recovers the same identity and cursor. Each chunk is at most 1 MiB of encoded
JSON and an append-only input without a new user submission. After input
validation, its new admission and adoption (`input/message`, `input/applied`,
`input/consumed`) commit in one batch with the cursor. These facts describe
accepting existing content now, never past execution. No model, tool or approval
runs. Repeated chunks compare the committed typed input; different values,
gaps and excess chunks fail. [Recorded inputs](../recorded-conversations.md)
define roles, effective context and legal message groups.

Files use SessionStore's immutable puts and ownership. Transfers use chunks of at
most 256 KiB; the existing per-file cap remains 16 MiB. Offsets, metadata and
repeated bytes must match. Completed bytes must match the digest before put.
Staging and completed file receipts survive restart. A receipt interrupted after
put may leave a private extra copy; finish releases unused owners for the existing
sweeper. The reply contains the new managed reference, never a legacy path.

Finish requires all declared chunks and files. It atomically seeds title metadata,
retains receipted file owners, inserts protocol ownership and marks the import
committed. Agents then publishes the directory identity. An interrupted reply is
recovered from the receipt. The Session subsequently restores and continues
normally. Incomplete imports remain unpublished and resumable; independent
completed objects are not rolled back. Deletion retains the reservation without
a foreign key to its Session. Later begin or finish returns `deleted` and cannot
recreate it. Invalid requests and storage failure remain distinct errors.

SessionStore owns the two staging/receipt tables in [native format v21](native-storage-format.md#recorded-inputs-and-import-coverage-v21).
Accepted storage and file operations settle before replying; cancelling migration
stops further submissions. Transport tests cover hidden partial Sessions,
duplicate/conflicting chunks, file interruption and restart, atomic publication,
continuation, deletion, unavailable model references and local-entry restrictions.
