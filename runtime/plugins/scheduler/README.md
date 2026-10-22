# Scheduler

Optional process and Session plugins implementing the accepted
[Scheduler contract](../../../docs/design/plugins/Scheduler.md). The process owns
jobs, windows, run tracking, reports and retention. The Session part offers
`schedule` to interactive leader Sessions. All execution uses the existing
`sessions`, `sessionPermissions`, `settings` and `pluginFiles` services.

## Install

Install the package into the runtime that resolves the configured rows. Put
`process.json` into `SessionTypeScriptConfig.process_rows` and `session.json` into
`rows`. Both structural configurations are `{}`. Their module declarations own
the dependencies; the supplied rows do not add duplicate loader gates. The
process row declares the `schedule` Settings source with user file
`schedule.json`, no project source and protected `/max_concurrent_runs`.

Provision `scheduler/` under the host's authorized plugin storage root before
enabling the row. The native file transaction does not create parent directories.
The package is not installed by the default CLI. Agents idle unloading
[#126](https://github.com/arcships/dim/issues/126) is required before create-mode
jobs ship.

## Public operations

The `schedule` tool supports `list`, `get`, `create`, `update`, `delete`,
`set_enabled`, `run_now` and `history`. Six protected local management methods
are exposed: `schedule.list/save/delete/set_enabled/run_now/history`. List and
history use existing report interruption; mutations drain admitted work.

Each authorizing create/update captures live authority and replaces the saved
origin and permission cap together, including rename-only updates. The result
shows the old and new authority and report destination. Management saves use
host authority and default to read-only execution. Tool saves use their saving
branch. Reuse checks the target's actual complete policy against both the saved
effective policy and the saving branch's current bound, without mutating it.

Jobs are global to this host, including across workspaces. Job edits use one
Settings document CAS; a conflict is returned without retry. Disabling or running
an existing job preserves its saved origin and cap. Accepted runs retain their
original authority, deadline and report destination through later edits.

The timer derives due windows from the enabled anchor, current wall clock and
durable history. It supports numeric five-field cron with IANA zones, fixed
minute intervals and one-off instants. Missing DST wall times are skipped and
repeated wall times run once. A gap of at least 60 seconds records one missed
window without catch-up. The current phase uses only the control node.

## Persistence and lifetime

`scheduler/runs.json` contains inner `{version:1,runs:[...]}` JSON in the native
S10 outer-v1 envelope. Missing files start empty. Invalid versions, malformed
records and incomplete receipt/ownership combinations fail activation with
`invalid_data`, preserving the original bytes. There is no migration.

Run admission writes `launching` before effects. Actual launch/reuse policy and
identity are recorded before input. The worker checks the same saved job revision
and fixed deadline before both admission and input, then tracks only its own
input through published history at one-second intervals. Recovery uses lookup
and history; it never relaunches or resubmits accepted work.

Tool-saved completed, failed, cancelled, timed-out and lost runs attempt one
Steer report to their recorded saving branch. Skipped/missed runs and every
management-saved run retain history only. Delivery failure records a diagnostic;
terminal records are never replayed. The run retains its scope through admitted
report completion and diagnostic writes before cleanup can finish.

Retention keeps active/unsettled runs and the newest 100 terminal runs per
existing job. Only successful launch receipts confer cleanup ownership. A
referenced reuse Session keeps its last launch record. Native deletion precedes
forgetting its final ownership record; failure keeps that record for retry,
including when no jobs remain. An unreadable Settings document does not remove
references. Native file transactions serialize the store; no JavaScript lock or
separate cleanup queue is added.

## Verification

Run from the repository root, using Node 24:

```sh
npm ci --prefix runtime/plugins/scheduler --ignore-scripts
node runtime/node_modules/typescript/bin/tsc -p runtime/plugins/scheduler/tsconfig.json
node --test runtime/plugins/scheduler/scheduler.test.mjs
```

| Public flow | Evidence |
| --- | --- |
| Save takeover, CAS failure, live permissions, shared cap/reuse vectors, tool restrictions and retirement | SDK service flows in `scheduler.test.mjs` |
| Cron DST gap/fold, enabled anchor, missed windows and one-off disable | SDK flows with a controlled wall clock |
| Held/consumed/applied input deadlines, uncertain withdrawal, late launch/reuse replies and revision rechecks | SDK execution flows |
| Restart lookup/history, single-attempt reports, admitted-report cleanup and deletion retry | SDK persistence/lifetime flows |
| Actual source defaults/protected fields, tool save, later live widening bounded by the saved cap, non-interactive execution, takeover, reporting and inner/outer-v1 records | `node_scheduler_saves_live_authority_runs_noninteractive_and_reports_to_the_latest_saver` |
| Invalid record startup, unchanged bytes, failed optional owner, retained hard dependencies and usable native Sessions | `node_scheduler_invalid_inner_records_preserve_bytes_and_do_not_block_native_sessions` |
| Agents index replacement keeps the owner and run loaded; owner reload preserves pending approval, input, deadline and anchor; completion/report occur once | `node_scheduler_owner_reload_tracks_the_existing_native_run_without_replaying_input_or_reports` |
| Native timer windows, DST gap/fold, enabled-anchor changes, one-off disable and missed-window collapse | `node_scheduler_native_clock_preserves_anchors_collapses_gaps_and_handles_dst_once` |
| Overlap/capacity skip and slot release; busy reuse waits for its own consumption and turn end | `node_scheduler_overlap_capacity_and_busy_reuse_follow_native_input_boundaries` |
| Held-input timeout withdraws only its input; consumed-input timeout stops its turn; user stop remains cancelled | `node_scheduler_native_deadlines_withdraw_held_input_and_stop_only_consumed_work` |
| Late actual launch/reuse/live-policy replies retain identity and policy, without input, withdrawal or stop after the deadline | `node_scheduler_late_native_admission_and_policy_replies_preserve_identity_without_input` |
| Complete host restart recovers committed input and does not replay terminal reports | `node_scheduler_complete_host_restart_reconciles_input_and_never_replays_terminal_reports` |
| Admitted native report writers drain on owner or Node replacement before deleted-job retention forgets their records | `node_scheduler_reports_drain_native_writers_before_row_or_runtime_cleanup` |
| Real filesystem deletion/replace failures preserve the last ownership record and retry with no jobs; external Sessions survive | `node_scheduler_retention_retries_real_delete_and_record_failures_with_no_jobs` |
| Fresh and pre-interrupt history prefixes preserve applied inputs without model consumption, deletion or another stop; later reuse consumption cannot change the terminal record | `node_scheduler_applied_inputs_interrupted_before_consumption_expire_without_another_stop` |
| A stale history prefix followed by genuine notHeld re-reads native deletion, completion or consumption before deciding | `node_scheduler_not_held_rereads_native_deletion_completion_and_consumption` |
| All shared cap and reuse vectors pass real authorizing saves, native launch policies and reused-policy checks | `node_scheduler_shared_policy_vectors_reach_native_save_launch_and_reuse` |
| Selected/other job edits, one-CAS conflict, frozen accepted prompt/authority/report target and accepted same-revision local writes | `node_scheduler_job_edits_cas_and_local_source_writes_preserve_accepted_snapshots` |

A prefix captured before a user interrupt can permit a withdrawal command at the deadline.
InputQueue returns `notHeld` for the applied input; the command does not imply an `input/deleted`
fact. The native check preserves this distinction and still requires no model call or extra stop.

The SDK suite has 20 flows; the fourteen native flows run with the repository's public
Node binding setup in CI. The test fixture wraps the real package's public ports
and uses Node's built-in test clock. Native SQLite publication and filesystem
permissions supply the report and retention faults; production interfaces are
unchanged. Strict typechecking includes this fixture.

Agents installs stored owners before publishing its index, as implemented in
[#327](https://github.com/arcships/dim/pull/327). The strict full-host flow verifies
recovery after crashes during the input reply, report input and report reply,
using real durable input and native report settlement without replay.

Remaining native gates from design Appendix D include unknown-input startup,
the complete report-outcome/failure matrix and remaining retention/source edge
cases. Existing SDK evidence is retained separately; this implementation stays
draft until those native gates pass.

Known contract limits remain: branch-addressed stop can interrupt a later turn;
adopted input cannot be retracted; a crash between terminal persistence and report
may omit the report; a launch reply lost before its Session identity is persisted
cannot be tracked or deleted by this plugin. These do not add core requirements.
