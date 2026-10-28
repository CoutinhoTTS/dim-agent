# Migration

`MigrationPlugin` is an on-demand process plugin shared by native clients. It owns
source reading, conversion, task progress and results. Native state changes go
through the target App Server's owners. It does not enable itself persistently,
register startup automation or run a separate migration daemon.

## Host interface

Mount `MigrationPlugin::new(target_home, theme_ids)` under an invocation context,
await it, and get `Migration`. `target_home` exists before mounting. The host calls
`validate_source_home(source, target)` before creating target resources or starting
an App Server. The target home must be outside the legacy home; a missing target
is checked through its existing parent without creating directories.

| Operation | Input and result |
| --- | --- |
| `inspect` | Legacy data home and stopped-source acknowledgement → database/table inventory |
| `run` | Legacy data home, acknowledgement, optional previous task ID and a target `ConnectionConfig` → durable `TaskReport` |
| `progress` | Watch receiver of the last persisted task state; task ID identifies each update |
| `list`, `report` | Durable task summaries or one complete report, without a target connection |
| `cancel` | Task ID → persist a cancellation request and return its latest state |
| `cancel_current` | Cancel this invocation's work and await settlement through its running operation |

The acknowledgement means that the user has exited every old CLI/Desktop using
the source and stopped its writers. Migration never terminates old processes.
The source is a product data directory, not the old source-code repository.
Inspection and metadata queries need no App Server. A run owns its connection;
the host supplies it only after resolving and checking the source.

Application registers no input consumer in `apply`. Accepted inspection and run
tasks belong to the plugin generation. Admission and close share a gate; cleanup
cancels and drains tasks before the owned FileStorage child is removed. A host
disposes the plugin after the operation and final report settle. Desktop can use
this interface without duplicating legacy conversion.

## Source and mapping

The reader accepts one `dimcode.sqlite` or `dimglobal.sqlite`, with the current
legacy CLI/Desktop schema, and an optional `schedule-tui.sqlite`. It copies the
database and committed WAL into a private target snapshot, then opens only that
copy in a read-only transaction. It never opens the source through SQLite or
creates source SHM files. Files are read beneath the authorized source home,
without following links. Snapshots are removed when the operation settles.

Session history retains stored order, roles, message IDs and source timestamps.
The effective context preserves compatible summaries and the uncompacted suffix,
excludes reverted/private/retry records and retains complete tool groups. Opaque
remote compaction without a compatible receiver blocks continuation explicitly;
it never becomes placeholder text or a fallback to all originals. Archived
Sessions are deferred as a whole.

Metadata, queues, task/run results, permission decisions and usage history are
readable in a Session-owned `legacy-history.json`; available background output and
Workflow artifacts are copied as immutable files. A model-excluded import note
links this archive. These records do not restore execution, grants or native usage
facts. File ownership follows ordinary Session deletion and fork.

Providers, accounts, Settings and Scheduler retain their owners' validation,
conflict, enablement and receipt rules. Missing Provider/model references are
retained. Shared CLI/Desktop settings choose Desktop values before target
comparison. Existing target values win, including false, empty values and explicit
clear. CLI settings use the same FileStorage leases as ordinary client writes;
clearing a client preference stores null. Unknown client fields and private plugin
state are reported and retained. Executable legacy Workflow definitions are not
registered as native declarative Workflows.

Web reads the shared version-1 `data/plugins/web/state.json` (the old CLI file
is used only when shared state is absent). Explicit Brave selection maps to
Web's search service; inactive adapters, account-dependent defaults and result
count preferences are reported without choosing a replacement. PluginManager
fills only absent, unlocked shipped selections; explicit target choices win.
Configuration fills precede normal plugin enablement reconciliation.

Skills disabled lists retain unchanged shared user IDs with one path component
and workspace IDs tied to an absolute `.agents/skills` root. Ambiguous nested
user IDs and host/plugin IDs defer the entire configured list, preserving a later
complete import. Shared Skills directories are reused without copying.

Command permissions map word-exact and word-prefix allow/deny rules. Connected
precedence groups containing unsupported rules or allow exceptions that native
deny priority cannot express are retained without approximation. Empty configured
rule lists remain explicit. At most 1024 rules are converted as one bounded input;
larger sources are reported without truncation. Historical remembered decisions
without an equivalent Session grant contract remain historical data.

Source JSON documents are bounded at 8 MiB. Recorded message/tool groups and files
use the [conversation receiver's limits](../../docs/design/appserver/conversation-import.md).
Oversized or unavailable inputs are reported before Session publication; no content
is silently truncated. Each archive is one bounded Session file.

## Tasks and retry

`<target>/migration/tasks/<task-id>/state.json` and per-object result files are
private format-1 records. They contain identities, progress, outcomes and fixed
diagnostics, never credentials or conversation bodies. A physical run lease
distinguishes active work from an interrupted process. A task left running without
that lease is reported as interrupted.
An interrupted directory created before its first state and result is ignored by
listing. Results without their task state remain an explicit integrity error.

Retry creates a new task with `retryOf` pointing to the previous task; its old
report remains queryable. It scans the selected source again and reconciles native
receipts. Completed objects are acknowledged without duplicate publication,
configured targets are preserved and deleted imports are not recreated. Changed
immutable source content conflicts rather than silently editing imported history.
Objects rejected before reservation can be corrected without stale failures from
the previous report. An existing immutable reservation requires the same content;
changed content conflicts even while it remains unpublished. Source paths do not
serve as durable object identities.

Progress stages identify their units: source/object counts, `file-bytes` and
`message-chunks`. Progress may be missed; persisted results and owner receipts are
the recovery authority. Local object failures continue independent objects and
produce a partial report. Global source/target storage failure stops the task and
keeps completed work. Cancellation is checked between handovers and waits for
accepted writes to settle. Completed, partial, failed, cancelled and interrupted
states remain readable after the plugin unloads.

Contracts: [Migration](../../docs/design/plugins/Migration.md),
[recorded inputs](../../docs/design/recorded-conversations.md),
[Settings fill](../../docs/design/appserver/settings-service.md#82-native-missing-field-import),
[Provider import](../../docs/design/appserver/provider-management.md#6-persistence-existing-sessions-and-migration),
[Scheduler import](../../docs/design/plugins/Scheduler.md#a31-configuration-import),
[account owners](../dim-app-server/README.md#managed-provider-contract) and
[MCP OAuth](../../docs/design/appserver/mcp-oauth.md#91-management-methods-namespace-mcp-local-transports-only).
