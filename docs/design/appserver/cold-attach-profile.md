# Cold first `session.attach`

Status: Draft for review ([#713](https://github.com/arcships/dim/issues/713)).
Scope: the first `session.attach` after daemon start for a stored, unloaded Session: what is measured,
the release-build cost split, and whether the restore before the reply needs a bound. It changes no
event, service, capability, protocol, configuration or storage format.

Related: [#601](https://github.com/arcships/dim/issues/601) (restore validation reuse), [#588](https://github.com/arcships/dim/issues/588)
(storage and IO audit), [#193](https://github.com/arcships/dim/issues/193) (restore profile),
[#712](https://github.com/arcships/dim/issues/712) (latest-turn window), [idle unload](idle-unload.md),
[restore-scoped shared history read](restore-shared-read.md), [sessions service §6](sessions-service.md#6-methods).

## 1. Summary

`session.attach` acquires a `SessionUse` before it dispatches. When the Session is not loaded,
`Agents::acquire` restores it before `attach` answers. That restore is the first large latency a
Desktop thread open pays; history paging starts only after the reply.

- **Measured.** In a release build over a disposable native store, the cold attach of a Session (its
  `main` branch) is 20.2 ms at 1,001 events, 76.7 ms at 5,001 events and 165.0 ms at 10,001 events
  (plain profile, median of five fresh Agents generations). The product assembly without MCP servers
  or skills is 82.7 ms at 5,005 events and 168.3 ms at 10,017 events. A `product` first restore, which
  also adds the product capabilities, is 110.2 ms at 5,010 events and 223.4 ms at 10,024 events. Warm
  attach is 0.13–0.22 ms. At 5k and 10k the restore split is stable: full validation is about 50%, the
  branch opening's first history read about 38%, the `branch/resume` append about 2%, and
  Session/branch assembly and scheduling about 7%.
- **Decision.** No contract or implementation change in this design. The cold attach cost is linear in
  stored history, both dominant steps are reads of the same immutable events, and the first open must
  validate. #601 removes the repeated validation on a re-acquisition; this design keeps the first
  open's full pass and states a provisional budget. The attach call's own work (Session record, title
  metadata, listener, approval replay, reply) is constant and small; the product's MCP and skill
  startup is not in this measurement and remains the open part of the issue's live-daemon numbers.
- **Unchanged.** The `session.attach` reply, its pinned `SessionInfo.sequence`, the live subscription,
  approval replay, leases, idle unload, history paging (#712), restore validation (#601), storage
  format, protocol, events, services and configuration.

The smallest implementation change this profile would justify is stated in §7: hand the events the
validation pass already decoded to the branch opening's first read, removing the second full decode.
It is not adopted here because the attach and product-startup measurements in §6 do not yet show a
budget breach.

## 2. What a cold first attach is

| Term | Meaning in this design |
| --- | --- |
| Cold first attach | The first `session.attach` for a Session id in a new App Server generation (daemon start or an Agents reload) with a native store, while the Session is not loaded and its store has not validated it in that generation |
| Re-acquisition | A later attach in the same generation after idle unload; [#601](https://github.com/arcships/dim/issues/601) makes it reuse the earlier validation while the directory is unchanged |
| Warm attach | A second attach in the same generation while the Session is loaded and leased; no restore |

The first request that names a Session gets a lease through `Agents::acquire`
([agents.rs:758](../../../crates/dim-app-server/src/agents.rs#L758)); `acquire` authorizes the
project, restores an unloaded Session ([agents.rs:795](../../../crates/dim-app-server/src/agents.rs#L795))
and announces it to Collaboration. `Connection::handle` performs that acquisition before it dispatches
the call ([connection.rs:632](../../../crates/dim-app-server/src/connection.rs#L632)); the attach
handler then builds the reply and installs the subscription
([subscription.rs:405](../../../crates/dim-app-server/src/connection/subscription.rs#L405)).

```text
client                Connection::handle          Agents::acquire        Agent restore                         attach
  | session.attach --------->|                          |                     |                                   |
  |                          | acquire(session) ------->|                     |                                   |
  |                          |                          | restore_session --->| branches (validation)             |
  |                          |                          |                     | Session and branch assembly       |
  |                          |                          |                     | opening reads, branch/resume      |
  |                          |                          |<--------------------|                                   |
  |                          | attach: Session record, title metadata, listener, approval replay, reply ---------->|
  |<------- reply -----------|                          |                     |                                   |
```

The restore path is the one [restore-scoped shared history read](restore-shared-read.md) describes:
validation decodes every stored event once, each branch opens with one shared decode of its history,
and the opening appends `branch/resume` before the Session is published. The attach reply is sent after
that. The measurement below separates the restore spans from the attach call's own work.

Excluded from "cold first attach": the client's connection and `hello`, daemon process startup, the
Desktop fold, and the history paging that #712 owns. They are outside the timed call.

## 3. Measurement

### 3.1 Scenario

The measurement uses a disposable native store, never the product home. One Session per size is grown
on `main` through real mock-model turns; no child branch, MCP server, skill or plugin is installed.
Sizes are the committed event count of `main`; the reported count is the head at the first cold attach.

| Item | Setting |
| --- | --- |
| Build | `cargo build --release --locked`, `debug-assertions = false` |
| Machine | AMD Ryzen 9 7945HX, 32 threads, 62 GB RAM, Linux 7.0.0, NVMe |
| Store | SQLite native format, fresh directory per run, `main` only |
| Profiles | `plain`: `defaults::install`, `defaults::mock_model_hosts`, `MockTools`; `product`: the product context window, prepare model and summary strategy that `restore_cost product` installs |
| Cold sample | A new `serve` generation on the same database; new store and Agents generation, Session unloaded; the Unix socket client is already handshaked; the timer starts when the `session.attach` frame is written and ends when its reply is read |
| Warm sample | The second `session.attach` on the same connection and Session, immediately after the cold one |
| Repetitions | Five per size, interleaved; median, minimum and maximum reported |

The process is reused across generations, so the OS page cache and the allocator are warm; a real
daemon start adds its own startup outside the timed call. `attach_cost` covers plain and product
assembly; `restore_cost` covers the product restore split; neither installs MCP servers or skills,
and neither depends on the Desktop client.

### 3.2 Commands

The restore split is measured by `restore_cost`:

```text
cargo run --release --locked -p dim-app-server --example restore_cost -- \
    product 1001,5001,6865,10001 5
cargo run --release --locked -p dim-app-server --example restore_cost -- \
    plain 1001,5001,10001 5
```

The cold attach total is measured by `attach_cost` (this PR):

```text
cargo run --release --locked -p dim-app-server --example attach_cost -- \
    plain 1001,5001,6865,10001 5
cargo run --release --locked -p dim-app-server --example attach_cost -- \
    product 1001,5001,6865,10001 5
```

The second argument is the event count per Session: each Session is grown on `main` through
`session.create`, `session.submit` and `session.idle` until its head reaches that count, so the actual
count reported is the head at the first cold attach. The third argument is the repetition count
(default 7). `plain` serves `defaults::install` with `defaults::mock_model_hosts` and `MockTools`;
`product` serves the product context window, prepare model and summary strategy over the same mock
model. Neither contacts a provider.

`ATTACH_COST_ROOT` selects the parent of the disposable home; the example creates a fresh UUID
directory below it, sets it to mode 0700, and removes it at the end. It must not name the product
home (`~/.dimcode`) or any existing store. The example's `session.submit` carries `branchId: "main"`
for protocol v3.

### 3.3 Cold attach

Release build at `103d1478`, disposable store, head after growth:

| Profile | Events | Cold attach median | Cold attach max | Warm attach median |
| --- | ---: | ---: | ---: | ---: |
| plain | 1,001 | 20.2 ms | 22.6 ms | 0.13 ms |
| plain | 5,001 | 76.7 ms | 82.2 ms | 0.20 ms |
| plain | 6,871 | 112.7 ms | 119.5 ms | 0.18 ms |
| plain | 10,001 | 165.0 ms | 175.8 ms | 0.22 ms |
| product | 5,005 | 82.7 ms | 90.8 ms | 0.14 ms |
| product | 10,017 | 168.3 ms | 210.9 ms | 0.22 ms |

Warm attach is below a quarter of a millisecond at every size. The plain cold total is in the same
range as the restore measurement in §3.4 at the same size (about 5 ms more at 1k; at 10k the attach
run is faster than the `restore_cost` run, whose profile includes the first idle and a different turn
mix). The attach call itself does not add a history-dependent step. The product rows install the
product capabilities but no MCP server or skill; that startup is the follow-up in §6.

### 3.4 Restore split

`restore_cost` first restore of each Session, release build, five repetitions per size. `Validation`
is the store's `branches` pass; `Opening reads` is the union of the store reads made while the branch
opens; `branch/resume` is the append until it is published; `Outside store` is the rest (Session and
branch assembly, capability rebuild, scheduling). Spans can overlap; the rows are medians, so their sum
is not exactly the total.

| Mix | Events | First restore | Validation | Opening reads | `branch/resume` | Outside store |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| product | 1,014 | 28.7 ms | 12.2 ms | 8.8 ms | 2.3 ms | 2.9 ms |
| product | 5,010 | 110.2 ms | 56.6 ms | 42.7 ms | 2.7 ms | 8.0 ms |
| product | 6,873 | 128.5 ms | 55.6 ms | 54.1 ms | 3.6 ms | 7.8 ms |
| product | 10,024 | 223.4 ms | 112.3 ms | 85.3 ms | 2.7 ms | 14.7 ms |
| plain | 1,006 | 15.6 ms | 5.7 ms | 6.3 ms | 2.3 ms | 2.1 ms |
| plain | 5,006 | 88.1 ms | 44.1 ms | 32.3 ms | 2.4 ms | 6.5 ms |
| plain | 10,006 | 195.2 ms | 99.2 ms | 67.3 ms | 2.7 ms | 14.3 ms |

The 1k/5k/10k first opens were measured on #601's branch (`5680d39d`); the first-open path is unchanged
from `main`, and a `main` run at `103d1478` reproduced the 1k and 5k shape (product 27.0 ms and 97.6 ms
median, maxima 36.5 ms and 115.0 ms). The 6,873 row is a `main` run. The split is the same in both:
validation is about half of the first restore, the opening read about 38%, assembly about 7%, and the
append about 2%. The product profile adds about 22 ms over plain at 5k (110.2 against 88.1), which is
the extra capabilities' rebuild, not storage.

### 3.5 What the tables do and do not cover

Covered: the App Server's attach path, the native store, full validation on first use, the branch
opening and its shared read, `branch/resume`, Session and branch assembly, and the attach reply and
subscription.

Not covered: project authorization before the restore, MCP server startup, skill/TypeScript binding,
a real model call, and a Desktop client. The issue's debug-build attach table (155–925 ms) and the live
daemon's 0.75–1.9 s are not reproduced by a release build at these sizes; the difference must be one
of the uncovered steps or a different event mix/machine. The next measurement isolates the product
startup rather than guessing it (§6).

## 4. Why the cost is what it is

- **Two decodes on a cold open.** Validation decodes every stored event once; the branch opening's
  first read decodes the same events again. #235/#239 made the opening's readers share one decode, not
  validation's. #601 removes the validation pass on a re-acquisition, but the first open keeps it by
  design.
- **Sequential, one connection.** Validation finishes before the branch is built, and the store has
  one connection; the two decodes cannot overlap. Both are linear in the event count, so the cold
  attach is linear: 20.2 → 76.7 → 165.0 ms for 1k → 5k → 10k. Nothing is superlinear after #235/#239.
- **Smaller steps.** The `branch/resume` append pays one commit and is roughly constant. Session and
  branch assembly, capability rebuild and scheduling are about 7% at these sizes. The attach call's
  own work is a metadata read for the title, a listener registration, pending-approval replay and the
  reply; it does not read history.
- **Warm is different.** Once loaded and leased, attach installs a listener and replies; the measured
  warm median is 0.13–0.22 ms, so the cost is entirely in the cold restore.

## 5. Decision

**No contract change, and no implementation change in this design.**

The issue asked for a measurement first and a bound only if one is needed. The release measurement
shows:

1. A cold first attach is one restore per Session per daemon generation. Idle unload makes the
   *second* use pay a restore too, and #601 removes its validation pass; the first use is the one this
   design owns.
2. The first open must validate. Skipping or deferring that changes when damage is reported and what
   `session.attach` may answer; #601 D8 and the issue keep it.
3. The two dominant steps are storage reads of immutable events. The opening read is already shared
   across the branch's readers; validation's decode is the only duplicate.
4. The cost is below a provisional budget on this machine:

| Size | Profile | Provisional cold-attach budget | Measured |
| ---: | --- | ---: | ---: |
| 5k | plain | 100 ms | 76.7 ms |
| 5k | product, no MCP/skill | 150 ms | 82.7 ms |
| 10k | plain | 200 ms | 165.0 ms |
| 10k | product, no MCP/skill | 300 ms | 168.3 ms |
| any | warm | 5 ms | 0.13–0.22 ms |

The product rows are `attach_cost product` without MCP servers or skills; the follow-up adds those
and closes the remaining gap to the issue's live-daemon numbers. The budget is a target for a later
change, not an acceptance that blocks this design.

5. No Desktop preload or cache is required: an attached client is a lease holder, and pinning Sessions
   would conflict with idle unload. A preload would move the same work before the user asks for it.

A lazy attach or a catalog-derived restore was considered and rejected for now: it would shift the
wait to the first history read rather than remove it, and it would change the reply's `sequence`, the
live subscription, pending-approval replay and the lease that keeps the Session loaded. That is a
separate protocol and lifecycle design, not a profile of the restore.

## 6. Evidence that would justify a later change

The remaining unknown is the product startup that neither `restore_cost` nor the plain `attach_cost`
includes. Measure it before changing the restore path:

| Observation | Conclusion |
| --- | --- |
| A product `attach_cost` with one MCP server and one skill, at 5k and 10k, stays within the §5 budget and storage remains about 50% + 38% | Close #713 as measured; no change |
| The product total exceeds the budget and MCP/skill startup is more than half of it | The change belongs to MCP/skills startup, not to attach or the restore path; open it with that owner |
| The product total exceeds the budget and the storage restore is still the largest step after startup is separated | Implement the candidate in §7 |
| The attach call's own work (record, title metadata, listener, approval replay) exceeds 20 ms | Optimize that work; it is not history-dependent |
| The issue's live-daemon 0.75–1.9 s is reproduced | Profile that environment; it is outside the release baseline measured here |

A cold attach that is already inside the budget at 1k, 5k and 10k does not justify a read-path change
whose only effect is to remove one decode.

## 7. Candidate: validation's decoded events to the opening read

Adopt this candidate only after the §6 product `attach_cost` with one MCP server and one skill at 5k
and 10k still exceeds the §5 budget and the storage restore is still the largest step. Do not
implement it before that measurement. The change is to stop decoding the same committed events twice
on a cold open.

- **Input.** The full validation pass already decodes every `StoredEvent` of every branch with its
  batch end, then drops them. Keep the decoded events and batch ends of the branch that is about to
  open, in the store, until that branch's first committed-history read.
- **Output.** The branch opening's first read of `1..=h` is served from that copy. Later reads, reads
  past `h`, and every other branch read from the store as today. The returned `StoredEvent` values,
  their `Arc<AgentEvent>` identities and batch ends are the same a store read returns.
- **Errors.** The same errors as the read it replaces: a range that ends inside an atomic batch still
  fails with `InvalidInput`; a storage failure that validation would report fails the restore before a
  copy exists; a read the copy does not cover falls back to `read_page`. A failed validation keeps no
  copy.
- **Bound.** The copy is one branch's history and is dropped when that branch's opening ends, like the
  opening's shared read ([restore-shared-read §4.2](restore-shared-read.md#42-what-the-copy-holds)).
  A Session with several branches must not retain all of them at once; if the store cannot bound it to
  the branch being opened (or, at most, `main`), do not adopt the change.
- **Contract.** The cache is inside the SQLite store's read path; `TimelineStore`'s signatures and the
  attach reply do not change. If the runtime's rule that validation's cache is released when validation
  ends ([timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#4-读取)) must be
  widened, that is one sentence raised to the runtime owner, not a new public contract.
- **What does not change.** Full validation and its checks, the attach reply and pinned sequence, the
  live subscription, approval replay, leases, idle unload, the history window, events, storage format,
  protocol and services.
- **Verification.** A flow test on a disposable store that fails the opening's store read after
  `branches`; the Session still restores and attaches, which shows the read was served from the
  validation copy. The existing corruption and restore flows keep their results.

This candidate removes about 38% of a cold first attach at 5k and 10k (42.7 ms of 110.2 ms and 85.3 ms
of 223.4 ms), leaving validation and the smaller steps. It does not help a re-acquisition, where #601
already skips validation and there is no copy to reuse.

## 8. Rule check

This is an App Server measurement and a decision about its attach path, not a plugin, capability or
event design. The applicable rules:

| Rule | Answer |
| --- | --- |
| [Principles §4](../../architecture/rust-agent/principles.md#4-通过时间线扩展行为) rule 1 (command events) | Not applicable: the measurement changes no state and writes no event |
| §4 rule 2 (handover) | Not applicable: nothing is handed over |
| §4 rule 3 (`before`) | Not applicable: no producer or draft changes |
| §4 rule 4 (no business meaning in core) | No core mechanism learns about attach; the measurement reads the store's existing spans |
| §4 rule 5 (atomic batches) | The measured reads and the candidate keep the existing batch-boundary rules |
| §4 rule 6 (model context) | Unchanged: no context input or insertion path is added |
| §4 rule 7 (public events and services) | No service or event changes; attach and the Session facade keep their contracts |
| §4 rule 8 (infrastructure) | No Timeline, Dispatcher, event envelope, assembly or facade change. The candidate would raise at most one sentence to the store's read contract |
| §4 rule 9 (facts in the Timeline) | No new fact; the measured work is a read |
| §1 (Capability and App Server boundary) | The restore and attach are the App Server's own path; no Capability is added |
| [ADR 002](../../adr/002-plugin-scope.md) D1 (scope follows state) | Process scope: the measurement runs in the App Server process; no new state |
| ADR 002 D2 (no wiring modules) | No new module or plugin |
| ADR 002 D3 (dependencies follow scope) | No new dependency |
| ADR 002 D4 (configuration) | None |
| ADR 002 D5 (language-neutral contracts) | No public contract changes; the candidate is internal |

## 9. Documentation changes

With this design:

- [the documentation index](../../INDEX.md) links this page.
- [restore-scoped shared history read](restore-shared-read.md) and the #601 design already assign the
  first open's validation to storage; this page owns only the cold attach measurement and decision.
- [idle unload](idle-unload.md) and [session history window](session-history-window.md) already state
  that an attach may restore the Session; they do not change.

With the measurement follow-up:

- Add the product `attach_cost` rows with one MCP server and one skill at 1k, 5k and 10k.
- If the candidate in §7 is adopted, update [timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#4-读取)
  for the copy's lifetime and link it from this page. No accepted ADR changes.

## 10. Acceptance and next steps

**This design.** The measurement scenario is defined, the release restore split is measured at 1k, 5k,
6.9k and 10k, and cold attach is measured at 1k, 5k, 6.9k and 10k in the plain profile. `attach_cost`
is in this PR and reproduces §3.3. The decision is to keep the first attach as a restore before the
reply and to set the provisional budget of §5.

**Follow-up measurement.** Run `attach_cost product` with one MCP server and one skill, five
repetitions at 1k, 5k and 10k, and report the attach total, the restore split, the attach-only span
and the plugin-startup span. Then:

- within the budget at all sizes: close #713 with this design and the numbers;
- over the budget for a startup step: open the change with that owner;
- over the budget for storage: propose §7 as its own implementation PR, with the flow test in §7 and
  the same measurement before and after.

No Desktop preload, cache or pinned Session is part of any step.
