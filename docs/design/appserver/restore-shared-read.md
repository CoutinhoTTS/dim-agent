# Restore-scoped shared history read

Status: Proposed, for team review. Phase 3 of [#193](https://github.com/arcships/dim/issues/193). Phase 1, a bounded
content cache shared across the pages of a read, merged as [#213](https://github.com/arcships/dim/pull/213); phase 2,
request message lists (native format v17), is [#224](https://github.com/arcships/dim/pull/224), and §15 of its
[design](request-delta-storage.md) leaves this phase to the runtime. The numbers here are measured on #224's branch.
The change is to runtime infrastructure ([principles §4](../../architecture/rust-agent/principles.md#4-通过时间线扩展行为)
rule 8), so it requires review by the runtime mechanism owner, the core mechanisms lane (eric8810,
[roadmap §3.5](../roadmap.md#35-按人按批次)). Author lane: Agents/storage (yyhhyyyyyy). Reviewers: eric8810, zerob13.
Scope: the committed-history read path of `Timeline`
([timeline.rs](../../../crates/dim-agent-runtime/src/core/timeline.rs)), which the branch scope starts sharing
([assembly.rs](../../../crates/dim-agent-runtime/src/assembly.rs)) and `TimelineContext` stops sharing
([timeline_context.rs](../../../crates/dim-agent-runtime/src/timeline_context.rs)). Design only; the implementation
follows review.

## 1. Summary

After v17 a Session restore still decodes each branch's history about a dozen times: storage validates it once,
TimelineTree scans it once, and every capability that rebuilds while the branch opens reads it again (§2). The reads
return the same immutable facts; only the decoding repeats.

- **Rule.** While a branch opens, its Timeline serves reads of the history the branch was built with from one decoded
  copy. The opening is the branch lifecycle's `Loading` phase: it starts when the branch scope builds the Timeline at
  the stored head `h`, and ends when the branch is activated (published), starts to close or unload, or its build is
  rolled back. The copy holds the branch's own events `1..=h` and, for a child branch, each ancestor prefix to its cut
  (§4).
- **Capabilities still read.** They fix `S = committed()` and read `local(..=S)`; nothing is pushed to them.
  The first read that needs the whole prefix decodes it with today's store read; readers that arrive meanwhile wait
  for it; each reader receives its own `Vec` of the same `Arc<AgentEvent>` values, as live dispatch hands every
  subscriber the same `Arc`. Events past `h` (terminals written during the opening, `branch/resume`) are read exactly
  as today. Reads outside an opening do not change.
- **Bounded in time and size.** The copy is filled once and never grows: it is one `local(..=h)` result, which every
  rebuilding reader holds today. It is dropped when the opening ends, so nothing of it survives publication, and an
  idle branch holds what it holds today (§5).
- **Failures.** A failed fill is not kept: each reader either receives the copy or the outcome of its own read, error
  included (§6).
- **Effect.** Measured on #224's branch at about 10,000 events, restore with first idle goes from 469 to 126 ms
  (plain), 527 to 141 ms (tools) and 496 to 170 ms (product). Full-history decodes per restore go from 12 to 3
  (validation, TimelineTree's scan and the fill). The peak heap of a restore falls to about a third, and live bytes at
  the first idle do not change (§8).

Unchanged: the Timeline as the per-branch time bus, the Dispatcher and its hot window, the capability rebuild
contract, the signatures and results of `local`, `history`, `context` and `committed`, event types and frozen
fixtures, capability and plugin code, storage and its format, and every protocol. No new event, service, method,
configuration or public API.

## 2. Problem and evidence

### 2.1 Restore cost after v17

#224 measured restore with first idle on v16 against v17 (its PR body; Apple M4, release, mean of two interleaved
rounds of seven): at about 10,000 events, plain 1,697 → 452 ms and tools 2,088 → 510 ms, with a full read of the
history taking 139 → 35 ms and 171 → 40 ms. The per-read cost fell four times; the number of reads did not change. At
10,000 plain events a v17 restore still spends 403 of its 469 ms in 11 full-history reads, 39 ms in validation and
27 ms elsewhere (§8).

### 2.2 What a restore is

A restore starts at `Agent::restore_session` and ends when the Session is published. What callers do afterwards is
outside it and unchanged: the first `idle` that `restore_cost` waits for makes no store call, and Agents' `acquire`
notifies its `SessionRestored` listeners (Collaboration) on the published Session. In between:

```text
Agent::restore_session(id)
  store.session_config(id)
  Agent::create(Restore), in its build task
    store.branches(id)                     storage validates every branch and row   ........ read A
    build_session: Session scope and Session capabilities (none of the measured ones reads branch history)
    TimelineTree::restore: one branch at a time, parents first
      for branch b at stored head h:
        store.read(b, 1..=h)               TimelineTree's own scan                   ........ read B
        child only: read_context(parent's Timeline, cut)  validates the cut          ........ read C
        TimelineFactory::build_until
          store.head(b)
          BranchScope: Timeline(b, ancestry, h); Dispatcher, not yet running       ─┐
          capabilities apply in dependency waves; each rebuilds from a read        │ opening of b
            (table below); some append settlement terminals                       │ (BranchLifecycle
          TimelineContext::start: dispatch begins                                  │  Loading)
        append branch/resume, wait until it is dispatched                          │
          opening handlers: ToolBinding reconciles, Prompt adopts its initial      │
          configuration; each reads the context                                   │
        finish_opening: opening receipts, then the committed fence                 │
        TimelineContext::activate: branch published                               ─┘
    Session::start, entries Ready, Session::activate: Session published
```

Reads of one branch's opening in `defaults::install`, the assembly `restore_cost` measures (product parts in the last
rows):

| Reader | Call | Range read | When |
| --- | --- | --- | --- |
| ContextPreparation | `local(..=S)` | own `1..=S` | apply, wave 1 |
| InputQueue | `local(..=S)` | own `1..=S` | apply, wave 1 |
| Permission | `history(Some(0))`, `local(..=S)` | ancestry, own `1..=S` | apply, wave 1 |
| ModelProvider | `local(..=S)` | own `1..=S` | apply, wave 2 (needs ContextPreparation) |
| Approval | `local(..=S)` | own `1..=S` | apply, wave 2 (needs Permission) |
| Prompt | `read_context(committed())` | ancestry and own `1..=S`, or from a checkpoint's `resume` | apply, wave 2 (needs ContextPreparation's `ConfigurationPublisher`) |
| ToolBinding | `read_context(sequence())` | as Prompt | apply, wave 3 (needs Prompt) |
| Loop | `local(..=S)` | own `1..=S` | apply, wave 4 (needs ToolBinding) |
| ToolExecutor | `local(..=S)` | own `1..=S` | apply, wave 4 (needs ToolBinding, Approval) |
| ToolBinding | `read_context(sequence())` | as Prompt, through `branch/resume` | opening receipt of `branch/resume` |
| Prompt, with an initial configuration (the product profile) | `read_context` twice, adopting and publishing | as Prompt, through `branch/resume` | opening receipt of `branch/resume` |
| Wait, BackgroundExec, Collaboration (product assembly, not in `restore_cost`) | `local(..=S)` | own `1..=S` | apply |

One traced restore of a plain Session at 10,002 events (`restore_cost` with `RESTORE_COST_SPANS`, every store call
with its range; readers inferred from their dependencies):

```text
ms from restore_session   store call                         reader
v17
    0-40                  branches                           validation, read A
   40-79                  read 1..=10002                     TimelineTree's scan, read B
   81-193                 read 1..=10002  x3, overlapping    ContextPreparation, InputQueue, Permission
  198-309                 read 1..=10002  x3, overlapping    ModelProvider, Approval, Prompt
  315-354                 read 1..=10002                     ToolBinding
  357-431                 read 1..=10002  x2, overlapping    Loop, ToolExecutor
  435-438                 append branch/resume
  438-476                 read 1..=10003                     ToolBinding, on branch/resume
  479                     Session published; the first idle follows at once, with no store call
prototype, same Session
    0-79                  branches, read 1..=10003           read A, read B
   82-120                 read 1..=10003                     the fill; every reader above is served from it
  122-127                 append branch/resume
  127                     read 10004..=10004                 ToolBinding's tail past h
  129                     Session published
```

- **Sequential or concurrent.** rutis applies a capability once its dependencies are active, so the rebuild reads
  run in four waves, one after another. The reads of a wave overlap in time but take turns on the store's connection
  segment by segment: three overlapping reads took about 111 ms each, against 38 ms for one alone. Branches open one
  after another: TimelineTree awaits each branch before reading the next one's events.
- **`S` is shared up to `h`.** The Timeline is built at `h`, the last stored sequence; `committed()` is `h` until
  something is appended. With unfinished work from before the restart, some capabilities append settlement terminals
  in `apply` (ToolExecutor, ContextPreparation, ModelProvider, InputQueue's owed decisions, BackgroundExec's wake
  inputs; [single-tree](../../architecture/rust-agent/single-tree.md#能力重载协议) row "失败判定"), so readers of a
  later wave fix a larger `S`. The opening handlers read through `branch/resume`. Every reader's range therefore starts
  with the same `1..=h` and differs only in a short tail written during the opening.
- **Hot window or store.** At construction the hot window is empty and the published sequence is `h`, so `1..=h` comes
  from the store. Events appended during the opening enter the hot window (the Dispatcher does not run until
  `TimelineContext::start`), and once the writer commits them they are also in the store. The Dispatcher itself reads
  only the hot window.

### 2.3 The multiplier

Every reader of the table decodes `1..=h` on its own. With v17 each such read costs about a full read of the history,
so the opening of a branch with `n` readers costs `n` full decodes on top of validation and the tree's scan. In the
plain and tools profiles `n` is 10 (nine in apply, one on `branch/resume`), so a restore decodes the history 12 times;
each rebuilding capability a product assembly adds is one more. Decoding dominates: in #193's profile the folds
themselves took at most 21 ms at 10,000 events.

## 3. Constraints

Each row names a contract, where it is written, and the mechanism that keeps it.

| Contract | Source | How this design keeps it |
| --- | --- | --- |
| The Timeline is each branch's time bus; capabilities consume it through the Dispatcher and rebuild by reading it | [time bus](../../architecture/rust-agent/time-bus.md), [timeline-storage §1](../../architecture/rust-agent/timeline-storage.md#1-结构与所有权) | The copy lives behind the Timeline's read functions. The Dispatcher, its hot window and subscriptions are untouched |
| Rebuild: fix `S = Timeline::committed()` in the rebuild gate, read `local(..=S)` (own events, no ancestor prefix); the read covers terminals the old generation wrote and has not published, without waiting for persistence or the Dispatcher | [single-tree, capability reload protocol](../../architecture/rust-agent/single-tree.md#能力重载协议) row "重建" | Capabilities call the same function with the same `S` and receive the same events. The copy covers only `1..=h`, which was stored before the Timeline existed; everything after `h`, including unpublished terminals, is read from the hot window and the store as today |
| `local`, `history`, `context`, `context_with_checkpoint`, `committed`: signatures and semantics | [timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#4-读取) | Unchanged. A slice is selected by sequence, so it holds exactly the events a read of that range returns; `through` and batch checks run as today |
| Events are facts once sequenced; order is the sequence only | [time bus §1, §2](../../architecture/rust-agent/time-bus.md#2-事件是总线上带序号的事实) | The copy depends on it: events `1..=h` never change, so a slice taken later equals a read taken later |
| Only immutable facts are shared; no push replay; no capability snapshot; no shared capability state | [time bus §2](../../architecture/rust-agent/time-bus.md#2-事件是总线上带序号的事实) (each state has one owner, decided from the events it consumed); [timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#有界读取与恢复扫描) (no business state for an optimization) | The copy holds `Arc<AgentEvent>` only, the value live dispatch hands every subscriber (`Committed(Arc<AgentEvent>)`). Each reader calls the read itself and folds into its own state |
| Bounded reads: segments, connection released after each | [timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#有界读取与恢复扫描) | The copy is filled by today's store read, with its segments and #213's bounded content cache |
| A checkpoint read does not scan request bodies before the snapshot's `resume` | [timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#有界读取与恢复扫描) | A read that starts after sequence 1 never fills the copy: it slices an existing copy or reads alone from `resume` |
| Restore caches are bounded; validation's cache is released when validation ends | [timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#有界读取与恢复扫描) "恢复" row, [§7](../../architecture/rust-agent/timeline-storage.md#7-验收) "分页与恢复" | The copy's size is fixed when it is filled: one `local(..=h)` result (plus, for a child, one ancestry read), which each rebuilding reader already holds today. It never grows and is dropped when the opening ends |
| The process keeps no dispatched events; how long a returned `Arc<AgentEvent>` lives is the caller's choice | [timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#使用方), last bullet | During its opening the Timeline holds one read result for its readers; after the opening it holds none (§15 adds this sentence with the implementation) |
| An idle branch's live bytes do not grow with steps | [timeline-storage §7](../../architecture/rust-agent/timeline-storage.md#7-验收) | The copy does not outlive the opening; live bytes at the first idle equal v17's (§8) |
| A read failure fails the operation that read, with the capability's failure terminal; it does not stop the Session's writes | [timeline-storage §6](../../architecture/rust-agent/timeline-storage.md#6-故障) | A failed fill is not kept; each reader receives its own read's error (§6) |
| Side effects are not replayed; a new generation takes over from an explicit sequence | [time bus §5](../../architecture/rust-agent/time-bus.md#5-副作用不重放) | Unchanged: capabilities still take over at their own `S` |
| Event types and frozen fixtures; plugin code | [principles §6](../../architecture/rust-agent/principles.md#6-验证), AGENTS.md rule 6 | Untouched |

## 4. Mechanism

### 4.1 Scope: the branch opening

The copy belongs to one branch's `Timeline` and lives exactly while that branch is `Loading`:

| Point | Where | Effect |
| --- | --- | --- |
| The branch scope builds the Timeline at `h` (`BranchScopePlugin::apply`) | assembly.rs | The opening starts, with the branch's ancestry and `h`; nothing is read yet. A branch with `h` = 0 and no ancestry has nothing to share and starts none |
| `TimelineContext::activate` | timeline_context.rs | The branch is published; the opening ends |
| `TimelineContext` close or unload starts (`finish`) | timeline_context.rs | The opening ends (failure, cancellation, Session close during the restore) |
| `Timeline::close` | core/timeline.rs | The opening ends; covers a build rolled back before a `TimelineContext` exists |

Branches open one at a time, so at most one copy of a Session exists at a time during a restore, and the last one is
dropped before the Session is published. The branch deadline (`TimelineFactory::build_until`) bounds an opening: when
it passes, the build fails and the Timeline closes.

The opening is the runtime's own unit; a restore is a Session whose branches all open with `branch/resume`. The same
opening also serves a branch created at runtime (TimelineTree `branch`, `create_main`) and a prepared Session (fork):
a new child has no events of its own but rereads its ancestry several times while it opens (§7). The runtime does not
need to know which of these it is building.

### 4.2 What the copy holds

- **Own part.** The events `1..=h` of the branch, as a vector of `Arc<AgentEvent>` in sequence order.
- **Ancestry part.** For a child, the events `1..=cut` of each ancestor with a cut above 0, in ancestry order.
- **Only events.** Traces and batch ends are not kept: the reads served from this path (`local`, `history`,
  `context`) return events only; `trace` keeps its own read. The `Arc`s are the store read's, so a reader that keeps
  an event keeps exactly what it would keep after its own read.
- **Fixed.** Each part is filled at most once and never extended; `h` and the cuts are fixed when the Timeline is
  built.

### 4.3 Serving a read

Every committed-history read of a Timeline goes through one internal path that splits the request into its hot window
part and stored parts; `local`, `history`, `context`, `context_with_checkpoint` and the wrappers on `Dispatcher`,
`Branch` and `Session` all reach it. Only the stored parts change:

```text
own stored part a..=b          (b <= published sequence, as today)
  no opening, or a > h ................................ store.read(a..=b)               as today
  a == 1 and b >= h ............ fill the own part once ─┐
  any other range, own part filled .....................─┴─> slice(own, a..=min(b, h))
                                                             then store.read(h+1..=b), if b > h
  any other range, own part not filled ................ store.read(a..=b)               as today

ancestor parts [(branch, a..=b), ...]
  every ancestor, each 1..=cut ...... fill the ancestry once ─┐
  any other set, ancestry filled ............................─┴─> one slice per segment
  any other set, ancestry not filled ..................... store.read_segments(...)    as today
```

- **Slices by sequence.** A slice takes the events whose sequence lies in the range (a binary search on
  `event.sequence`), so it is what the store returns for that range: one event per sequence, merged output rows
  already split into their events.
- **Past `h`.** The part after `h` is never served from the copy. It is read from the hot window and the store by
  today's code, so a reader with a larger `S` sees the terminals written during the opening exactly as today.
- **Sub-ranges.** `local(a..=b)` with `a > 1`, a checkpoint read from `resume`, and the checkpoint anchor read are
  slices once the copy exists, and plain reads before. They never fill it: a read that starts after sequence 1 does
  not cause the prefix before it to be decoded.
- **Different `S`.** Readers fix their own `S`; the copy only replaces the stored prefix `1..=h` that every such range
  starts with.

### 4.4 Single flight

Each part is filled at most once, by the first read that needs all of it; readers that need it while the fill runs
wait for that fill instead of starting their own. In the prototype this is `tokio::sync::OnceCell::get_or_try_init`,
whose semantics are the ones this design needs:

- The filling reader runs the store read with no lock held; waiters wait on the cell, not on the Timeline's data lock,
  so appends and dispatch never wait for a fill.
- If the fill fails, the cell stays empty, the filling reader receives its error, and the next waiter runs its own
  read (§6).
- If the filling reader is cancelled (its apply future is dropped), the next waiter fills.
- A fill waits only for the store, and no store call waits for a Timeline reader, so a waiter can never be waiting on
  itself, whatever rebuild gate it holds.

### 4.5 Release

The Timeline drops its reference to the copy when the opening ends (§4.1). A reader that obtained the copy before
that finishes slicing and drops its own reference, so the copy's memory is freed when the last such reader returns.
After the opening, reads go to the store as before. Nothing else holds the copy: it is private to the Timeline, never
handed to a capability, and never cached by key outside the Timeline.

### 4.6 Readers outside an opening

Unchanged, because no copy exists: capability reloads in an active branch, model steps (`context`,
ContextPolicy's `local(through+1..)`), Prompt and configuration publication, TimelineTree's `branch` validation on a
parent, the App Server's history pages and summaries, collaboration and export.

## 5. Residency

Today each reader holds its own decoded history from its read until its fold returns, and readers of one wave overlap,
so a restore's peak holds several decoded histories at once, plus the opening handlers' context reads after the
waves. With the copy the opening holds one decoded history from the first fill until the branch is published; each
reader adds a vector of pointers (8 bytes per event) while it folds.

Measured with a counting allocator in `restore_cost` (§8): the peak heap above the level before the restore, during
the restore and its first idle, in MB.

| Mix | Events | `main` (v16 + #213) | #224 (v17) | #224 + prototype |
| --- | --- | --- | --- | --- |
| plain | ~1k | 3.9 | 5.4 | 2.4 |
| plain | ~3k | 18.2 | 17.8 | 6.6 |
| plain | ~10k | 110.7 | 95.9 | 32.7 |
| tools | ~1k | 4.4 | 5.9 | 2.6 |
| tools | ~3k | 21.1 | 20.1 | 7.5 |
| tools | ~10k | 134.7 | 112.5 | 38.1 |
| product | ~1k | 4.9 | 6.8 | 3.0 |
| product | ~3k | 18.0 | 19.7 | 7.3 |
| product | ~10k | 65.3 | 69.2 | 23.7 |

At the first idle the heap is 0.30 to 0.33 MB above the level before the restore in every build, mix and size, and
0.10 MB after the Session unloads. The prototype counted no opening alive at the first idle in any restore.

- **No worse than today.** The copy is one of the results every reader already materializes; the readers' own
  results become pointer vectors. A reader that keeps an event keeps the same `Arc` it would keep today.
- **Nothing survives publication.** The copy is dropped at activation; live bytes at the first idle are v17's.
- **Long-session residency.** An idle branch holds no copy, so its live bytes do not grow with steps
  ([timeline-storage §7](../../architecture/rust-agent/timeline-storage.md#7-验收)).
- **Child branches.** A child's opening holds its own events and one ancestry read: the size of one `context` read of
  the child, which Prompt and ToolBinding each materialize today.

## 6. Failure semantics

| Case | Behaviour |
| --- | --- |
| The fill's store read fails | The filling reader receives its error. The cell stays empty; each waiting reader then runs its own read, one after another, and receives its own result. A capability whose read fails fails its `apply`, the branch build fails and is rolled back as today, and the Timeline's close ends the opening |
| A suffix read past `h` fails | That reader receives the error, as today |
| The filling reader is cancelled | The next waiter fills; nobody receives a partial copy |
| Storage commit failure during the opening | Unchanged: the part past `h` follows today's hot window and store rules; `1..=h` was committed before the Timeline was built |
| The opening ends while a reader holds the copy | That reader finishes with the events it was served; later readers read the store |
| Corrupt rows | `TimelineStore::branches` validated every row before the build (read A); a fill that hits damage fails like any read |

**Same events per reader.** A reader served from the copy receives the events `store.read` returns for `1..=h` (the
fill was such a read), cut to its range by sequence, followed by what today's code returns after `h`. The events
`1..=h` are committed, and storage never changes a committed event: it appends events, extends an output row only
with later events, and its upgrades rewrite no business row ([storage format](native-storage-format.md)). The reader
therefore observes the event set its own read would return. The only difference is exposure to transient I/O errors: a
reader served from a copy filled a moment earlier does not repeat that I/O.

## 7. What it removes and what remains

Reads of `main` in one restore of a `main`-only Session, from the traces of §2.2 and §8:

| Read | Plain and tools today | Product today | With the copy |
| --- | --- | --- | --- |
| A. `TimelineStore::branches` (`validate_session`) | 1 full | 1 full | 1 full; storage's own pass, see below |
| B. TimelineTree's restore scan | 1 full | 1 full | 1 full; capability code reading the store before the Timeline exists |
| `local(..=S)` of ContextPreparation, InputQueue, Permission, ModelProvider, Approval, Loop, ToolExecutor | 7 full | 7 full | 1 full, the fill; the other readers get slices |
| Prompt and ToolBinding in apply (`read_context`) | 2 full | 2 from a checkpoint's `resume` | slices |
| Opening handlers on `branch/resume` | 1 full (ToolBinding) | 3 from `resume` (ToolBinding, Prompt twice) | slices, each with a one-event tail |
| Wait, BackgroundExec, Collaboration (product assembly, not measured) | 3 full | 3 full | slices |

A checkpoint read here is two store calls: the anchor read from the snapshot's sequence to the cut and the context from
`resume`, 354 and 212 events at 10,000 product events.

A child branch adds, today, its own reads as above and five reads of its ancestry: TimelineTree's cut validation on the
parent (read C), Permission, Prompt and ToolBinding twice. With the copy it adds its own part once and its ancestry
twice (read C and one fill). Traced on v17 for a child cut at 3,002 events of `main`: its own events were read 11 times
and `main`'s prefix 5 times; with the prototype, twice each (own: scan and fill; prefix: read C and fill), plus a
one-event tail.

**Validation hand-over (read A) is out of scope.** Storage validates every branch before the runtime builds anything,
page by page, with a bounded cache released at the end of the pass
([timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#有界读取与恢复扫描), "恢复" row). Handing its
decoded events to the runtime would hold every branch's history from validation until the last branch opens, change
what the `TimelineStore` port returns or keep state between its calls, and contradict the bounded-cache convention.
Making validation itself cheaper is storage's work and needs no runtime change.

**TimelineTree's scan (read B) stays.** It decides whether to start the branch at all (an interrupted creation, a
failed create recorded on the parent) before its Timeline exists, and it is capability code, which this design does
not change. Whether that scan can be narrowed is a question for TimelineTree's owner (§16).

## 8. Measured effect

Release builds on one Apple M4, disposable stores under an isolated home, no other build running. Three builds of
the same `restore_cost` harness: `main` at 2946f25d (format v16 with #213), #224 at 29d3450b (v17), and #224 with the
prototype of this design (uncommitted). The harness adds to #224's version a counting allocator, the store calls of a
restore with their ranges, an optional child branch and an optional pause before the create; v16 Sessions were grown with the `main` build and v17
Sessions with the #224 build, which the prototype restores too. Two rounds per mix, in the orders main, v17,
prototype and prototype, v17, main; seven restores per size in each; the tables show the mean of the two medians.
Round medians agreed within 3% at 10k. Nothing here is asserted in tests.

Restore with first idle, ms:

| Mix | Events | `main` | #224 | #224 + prototype | Prototype / #224 |
| --- | --- | --- | --- | --- | --- |
| plain | ~1k | 53.7 | 40.3 | 19.0 | 0.47 |
| plain | ~3k | 240.6 | 118.3 | 35.1 | 0.30 |
| plain | ~10k | 1,750 | 469 | 126 | 0.27 |
| tools | ~1k | 62.2 | 45.0 | 17.0 | 0.38 |
| tools | ~3k | 284.3 | 132.6 | 38.6 | 0.29 |
| tools | ~10k | 2,136 | 527 | 141 | 0.27 |
| product | ~1k | 65.8 | 62.0 | 20.5 | 0.33 |
| product | ~3k | 155.7 | 146.9 | 51.0 | 0.35 |
| product | ~10k | 524 | 496 | 170 | 0.34 |

Store reads per restore, besides validation (the same at every size for a mix): the number of read calls, and the
events they returned over the Session's events, that is, full-history reads:

| Mix | `main` and #224 | #224 + prototype |
| --- | --- | --- |
| plain, tools | 11 calls, 10.9 to 11.0 | 3 calls, 2.0 |
| product | 18 calls, 11.2 (1k), 8.7 (3k), 8.3 (10k) | 8 calls, 2.0 |

The prototype's calls are TimelineTree's scan, the fill and one-event tails past `h` (one in plain and tools, six in
product, where three opening handlers each read from a checkpoint twice). In every restore three readers arrived while
the fill was running and waited for it; 10 (plain, tools) and 17 (product) reads were served from the copy.

Where a plain restore at ~10k spends its time, ms (the store's spans; reads overlap, so their wall time is counted
once):

| | Validation | Reads | `branch/resume` append | Outside the store | Total |
| --- | --- | --- | --- | --- | --- |
| `main` | 147.8 | 1,577.8 | 4.2 | 20.7 | 1,750 |
| #224 | 39.1 | 403.0 | 4.3 | 22.6 | 469 |
| #224 + prototype | 39.0 | 75.0 | 4.3 | 7.4 | 126 |

What is left is about a third each: validation (read A), TimelineTree's scan (read B) and the fill. Bytes per event
are unchanged (411 against 410 at plain 10k), as the store is unchanged.

A child branch (one traced restore of a plain Session with a child cut at 3,002 events of `main`; counts only, see
§7): `main`'s prefix was read 5 times on #224 and twice with the prototype, the child's own events 11 times and twice.

**Session create.** In the same rounds a Session create took 9 ms (plain, tools) and 39 ms (product) on `main` and
#224, and 21 to 24 ms and 51 ms with the prototype, though a new `main` starts no opening (`h` = 0, no ancestry). The
cause is the store's writer, which starts its rounds at least 20 ms apart
([storage/timeline.rs](../../../crates/dim-app-server/src/storage/timeline.rs),
[timeline-storage §3](../../architecture/rust-agent/timeline-storage.md#3-写入与发布)): the prototype's
restore ends within 20 ms of its own `branch/resume` round, so the create's first round waits. With a 40 ms pause before
the create, the prototype's create took 9 ms (plain) and 36 ms (product). The same pacing lengthens the prototype's
`branch/resume` append in the small restores (8.5 ms against 4.1 at plain 1k), so the 1k ratios understate the gain.

## 9. Impact surface

Following [principles §3](../../architecture/rust-agent/principles.md#3-正交): the target is zero changes outside the
runtime's Timeline and zero new public content.

| Module or content | Change | Reason |
| --- | --- | --- |
| `dim-agent-runtime` `core/timeline.rs` | The opening's copy in the committed-history read path; begin and end functions (crate-private) | The read cost is the Timeline's own |
| `dim-agent-runtime` `assembly.rs`, `timeline_context.rs` | `BranchScopePlugin` begins the opening; `activate` and the start of close or unload end it | The branch lifecycle already delimits `Loading` there |
| `testing` feature | A count of openings that hold a copy, for the release checks of §14, like #224's node-load counter | After a failed or cancelled build no public call can reach the Timeline, so its release is not observable otherwise; the read counts need no hook (a counting store) |
| Dispatcher, hot window, `TimelineStore`, storage, event types, fixtures, capabilities, plugins, protocols | None | §3 |
| Public content added | None | No event, service, method, wire field or configuration |

## 10. Rule check

| Rule | Answer |
| --- | --- |
| [Principles §4](../../architecture/rust-agent/principles.md#4-通过时间线扩展行为) rule 1 (command events for others' state) | Not applicable: no state of another module changes |
| §4 rule 2 (handover) | Not applicable: nothing is handed over; readers still read |
| §4 rule 3 (before callbacks) | Unchanged; the copy holds committed events, which before callbacks never see again |
| §4 rule 4 (no business meaning in core) | The copy is keyed by branch and sequence ranges only; it reads no event kind and has no special case for any capability |
| §4 rule 5 (atomic batches) | Unchanged: `h` is a batch boundary (the published sequence), reads return the same events, and the batch checks of a read run as today |
| §4 rule 6 (model context) | Unchanged: the same events, so the same context |
| §4 rule 7 (public events and services) | Capabilities keep reading through the Timeline service; none reads another's state, and none is given another's results |
| §4 rule 8 (infrastructure) | A change to the Timeline, raised by the Timeline's own read cost and closed inside the runtime: §11 |
| §4 rule 9 (only decision facts in the Timeline) | No new fact; the copy is process memory |
| §1 (Capability and App Server boundary) | Inside the runtime; no capability or App Server change |
| §3 (orthogonality) | No other module changes; §9 lists the impact surface |
| [ADR 002](../../adr/002-plugin-scope.md) D1 (scope follows state) | Branch scope: the copy belongs to one branch's Timeline and lives with its opening |
| ADR 002 D2 (no wiring modules) | No new module or plugin |
| ADR 002 D3 (dependencies follow scope) | No new dependency |
| ADR 002 D4 (configuration) | None |
| ADR 002 D5 (language-neutral contracts) | No contract changes; a TypeScript part reading through `Dispatcher::local` is served the same way |

## 11. Requirements raised to mechanism owners

None to rutis. The change itself is to the runtime, so it answers rule 8's items here for the runtime mechanism owner:

| Item | Content |
| --- | --- |
| Blocked by | Nothing in a plugin. The requirement comes from the Timeline: during an opening, each reader of the same immutable prefix pays the store's decode, and the rebuild contract ([single-tree](../../architecture/rust-agent/single-tree.md#能力重载协议)) makes every rebuilding capability such a reader |
| Existing mechanisms considered | The hot window holds only undispatched events and is empty at a restore. Storage's caches are bounded per read call and cannot span calls by convention. Capability-side changes (snapshots, kind-filtered reads, pushing events) are rejected in §13 |
| Generic capability | A Timeline serves reads of the history it was built with from one decoded copy while its branch opens. No business meaning: branch, sequence ranges and the lifecycle phase only |
| Effect on sequence, batch, interrupt, reload | Sequence and batch: none; reads return the same events. Interrupt: none; an interrupt during an opening is an event past `h`. Reload: a capability reloaded while its branch is still opening reads the same copy; one reloaded later reads the store as today |
| Verification | Inside the runtime and its flows (§14) |
| Owner and issue | Runtime, core mechanisms lane (eric8810, [roadmap §3.5](../roadmap.md#35-按人按批次)); [#193](https://github.com/arcships/dim/issues/193) |

## 12. Decisions

| # | Decision | Rejected alternative | Why |
| --- | --- | --- | --- |
| D1 | The copy lives with a branch's opening (`Loading`) | Scoped to a Session restore, released at Session publication | Branches open one at a time, so a branch scope holds one branch's history; a Session scope would hold every branch's until the last opens. The branch lifecycle already marks the opening in the runtime, and the runtime need not know whether a branch is restored, created or prepared |
| D2 | The seam is the Timeline's committed-history read path | A forwarding `TimelineStore` for the Session's restore; wrappers in `Dispatcher::local`, `Branch` and `Session` | A store wrapper would also serve TimelineTree's scan (read B), but it would intercept the storage port for the Session's whole life, keep state between its calls and change what TimelineTree's direct store read means. Facade wrappers would cover some readers and not others (`read_context` goes through the Timeline directly) |
| D3 | The copy holds events, not traces or batch ends | The store's `StoredEvent`s | The paths served return events only; traces would add memory and deep copies for no reader |
| D4 | A part is filled only by a read that needs all of it | Fill on any read | A checkpoint or tail read must not decode the prefix before it (timeline-storage §4) |
| D5 | The copy is fixed at `h`; reads past `h` are today's reads | Append the tail as readers read it | The tail is a few events written during the opening; extending the copy would add a second fill rule and mutable shared state for no measured gain |
| D6 | A failed fill is not kept; waiters read for themselves | Share the fill's error with its waiters | Each reader's outcome stays the outcome of its own read (timeline-storage §6), and a transient failure does not fail readers that could succeed |
| D7 | A child's ancestry is shared like its own events | Own events only | A child's opening reads its ancestry about four times (Permission, Prompt, ToolBinding twice), at creation and at restore; the same rule covers it at no extra mechanism |

## 13. Rejected alternatives

- **Replaying history to capabilities** (the Dispatcher, or a restore step, hands each subscriber the events to fold).
  Capabilities would stop reading at their own `S`; the rebuild contract and the Dispatcher's live-only role change.
- **Capability snapshots for rebuild.** Business state stored for an optimization, in every plugin
  ([timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#有界读取与恢复扫描)), and a second source of
  truth beside the events ([time bus §3](../../architecture/rust-agent/time-bus.md#3-总线只承载影响后续行为的结果)).
- **Sharing rebuilt state between capabilities** (for example one fold that several capabilities read). State has one
  owner, which decides only from the events it consumed ([time bus §2](../../architecture/rust-agent/time-bus.md#2-事件是总线上带序号的事实)).
- **A store-wide decoded-event cache.** Residency would grow with every Session loaded or read recently, against the
  idle-branch acceptance; phase 2 rejected it for the same reason ([request delta storage §17](request-delta-storage.md#17-rejected-alternatives)).
- **Validation handing its result to the runtime.** §7.
- **Kind-filtered reads** (each capability asks only for the kinds it folds). Changes the read API and every
  capability's rebuild, and the store still decodes each row to filter it.
- **Keeping the copy until the Session publishes, or for some time after.** More residency for no reader: after
  activation reads are rare and served as today.

## 14. Acceptance

The implementation PR passes all of these. Behaviour checks are deterministic and run in the existing flows
(`tests/integration`, the runtime's tests); timing and size targets are measured with `restore_cost` in release
builds and reported in the implementation PR, not asserted in tests.

**Behaviour (CI):**

1. **One decode per opening.** Restoring a Session of `defaults::install` through a counting store, each branch's
   own history `1..=h` is read from the store at most twice during the restore (TimelineTree's scan and one fill),
   whatever the number of rebuilding capabilities; the reads past `h` cover only events written during the opening.
   With a child branch, its ancestry is read once by its opening (plus TimelineTree's cut validation on the parent).
2. **The same events per reader.** The existing restore, reload and branch flows pass unchanged, including restores
   with unfinished work (settlement terminals written during the opening, so readers fix different `S`), a
   compaction checkpoint (a read from `resume`), and a child branch.
3. **Release.** After `restore_session` returns, no opening holds a copy (`testing` counter), and a later
   `local(..)` reads the store. After a restore that fails (a branch whose rebuild read fails, an injected read
   failure), and after a restore cancelled by `Agent::close` or the Session's close, no opening holds a copy.
4. **Failure.** An injected failure of the fill read fails the restore with the same storage error as today; the next
   restore succeeds.
5. **Readers outside an opening.** A capability reloaded in an active branch reads the store.
6. **Frozen contracts.** `crates/dim-agent-types/tests/fixtures/` and the `*_contract.rs` tests are unchanged.

**Measured (reported in the implementation PR):**

7. Restore with first idle, `restore_cost` plain, tools and product at about 1k, 3k and 10k events, against v17 on the
   same machine: never slower than v17; at 10k, at most 35% of v17's in plain and tools and at most 40% in product
   (the prototype measured 27%, 27% and 34%).
8. The peak heap of a restore is at most v17's at every size and mix (the prototype measured about a third at 10k),
   and the heap at the first idle is v17's within 0.01 MB.

## 15. Changes to other documents

With this design: the [documentation index](../../INDEX.md) links this page.

With the implementation, each keeping its meaning:
[timeline-storage](../../architecture/rust-agent/timeline-storage.md) §4 "使用方" adds that while a branch opens its
Timeline serves reads of the history it was built with from one shared copy, released when the branch is published,
closed or rolled back, and §5's "恢复" row says the capabilities' `context` or `local` reads share it; the rebuild row
of [single-tree](../../architecture/rust-agent/single-tree.md#能力重载协议) stays as written. When #224 has merged,
§15 and §20 of [request delta storage](request-delta-storage.md) link here.

## 16. Open questions for reviewers

1. **Child ancestry (D7).** Sharing a child's ancestry also speeds up branch creation at runtime. Keep it in this
   change, or limit the first implementation to the branch's own events?
2. **TimelineTree's restore scan (read B).** It reads each branch's whole history to find `branch/start`, the first
   queued input and `branch/create-failed`. Should its owner narrow it, or should it read after the Timeline is built
   so the opening's copy serves it?
