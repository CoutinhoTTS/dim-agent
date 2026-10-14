# Session access

Status: Draft. Overall design: [overview §3.2](../overview.md#32-capabilities-and-process-plugins), [§3 methods B and E](../overview.md#3-integration-methods), [§12 P0](../overview.md#12-phases). Phase: P0. Language: Rust.

The public interface through which process plugins read a Session's fixed published Timeline prefix and write input into it. It narrows existing public APIs (`Session::input/history/local_page/sequence/idle/stop`, `AgentIndex::acquire`, `session.input_lookup` semantics) into one process service with source attribution. The contract knows no caller: any process plugin uses it, and callers depend on this contract, not on the plugin that provides it. Precedent: Collaboration, which already submits, stops, waits for idle and reads pages on the peer Session's public interface ([collaboration.rs](../../../crates/dim-app-server/src/collaboration.rs)).

## 1. User flows

1. A caller receives `turn/end` from the [broadcast subscription](session-broadcast-subscription.md), reads the branch prefix through the published head, finds its own keyed block still in effect and no interrupt after the turn, and writes a Queue input carrying its source marker.
2. A caller acts on an unloaded Session: it acquires the Session (restoring it), writes user content with its marker block in Queue mode and a `submission_id`, then releases the lease.
3. A caller waits for `idle` on a Session it started, reads the final prefix, and reports to another Session with Steer input.
4. A write fails with an uncertain outcome (reload window); the caller looks the `submission_id` up before retrying.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Read of a fixed published prefix; paged local reads | Capability state (`Session::get/on`, `StateCell`s): process plugins never read it |
| Input writes (modes, raw blocks, keys, user content) with source attribution | Arbitrary events, `stop/interrupt` other than `stop`, configuration, branch creation |
| Loading an unloaded Session through AgentIndex | Session creation (Agents supplement) and deletion (SessionStore supplement) |
| Submission identity and lookup | Client protocol (`session.submit` stays operator-content only) |

## 3. Scope and dependencies

**Contracts defined here** (owner: AgentsPlugin, next to `AgentIndex`): the `SessionAccess` service, `SourceAccess`, `SessionLease`, `SourceInput`/`RelayInput`, the source marker and key namespace conventions, `Lookup` and `AccessError`. **Contracts depended on**: the runtime `Session`/`Branch` public facades, `AgentIndex` loading, the `input/message` data contract and its consumption contract (InputQueue modes, Loop consumption, history projection of raw blocks, keys and `retire`). Callers depend on this contract only; it carries no caller-specific rule. The namespace of every marker and key is the producing plugin's own source name.

| Part | Scope | Injects | Provides | Status |
| --- | --- | --- | --- | --- |
| `AgentsPlugin` | Process | existing | `AgentIndex` (existing), `SessionAccess` (new, same plugin, so its lifecycle is AgentIndex's) | Supplement |
| Runtime `Session` / `Branch` facades | Session / branch | — | `input`, `history`, `local_page`, `sequence`, `publication`, `idle`, `stop`, `branches` | Existing ([agent.rs](../../../crates/dim-agent-runtime/src/agent.rs), [branch.rs](../../../crates/dim-agent-runtime/src/branch.rs)) |
| InputQueue, Loop, WaitBranch | Branch | existing | consume `input/message` | Existing |
| Consumers | Process | `Arc<SessionAccess>` | — | Proposed per plugin |

## 4. Events

None new. Writes produce the existing `input/message` (`InputMessageData { submission_id, user, blocks, mode }`); consumption produces the existing `input/applied` (InputQueue) and `input/consumed` (Loop). `stop` produces the existing turn-scoped `stop/interrupt`. Declaring `input/message` does not make Session its exclusive producer: any plugin writes it through this port ([time bus §2](../../architecture/rust-agent/time-bus.md)).

## 5. State

| State | Owner | Rebuild |
| --- | --- | --- |
| Input queue, applied/consumed inputs | InputQueue, Loop (`StateCell<InputSnapshot>`, `LoopSnapshot`) | From the branch's local history on reload (existing) |
| Leases pinning loaded Sessions | `SessionAccess` (wraps `SessionUse`) | Runtime only; dropped on caller unload |
| Keyed block effect, model context | History projection ([history.rs](../../../crates/dim-agent-capabilities/src/history.rs)) | From the fixed prefix and compaction facts |
| Caller progress (e.g. pending `submission_id`) | Caller, process storage | Caller's document |

## 6. Interfaces

```rust
/// Process service; key TypeKey::of::<SessionAccess>(). In-process only, never exported as RPC.
impl SessionAccess {
    pub fn source(&self, name: &'static str) -> SourceAccess;            // binds attribution (A1)
}
impl SourceAccess {
    pub fn loaded(&self) -> Vec<SessionRef>;                              // AgentIndex::hosts + Agent::sessions
    pub fn get(&self, session_id: &str) -> Option<SessionLease>;         // loaded only (AgentIndex::session)
    pub async fn acquire(&self, session_id: &str) -> Result<SessionLease, AccessError>; // may restore (AgentIndex::acquire)
    pub async fn stored_page(&self, session_id: &str, branch: &str, after: u64, limit: usize,
        through: Option<u64>) -> Result<Page, AccessError>;               // no load; store head when unloaded
}
impl SessionLease {                                                        // pins one loaded incarnation
    pub fn session_id(&self) -> &str;  pub fn agent_id(&self) -> &str;
    pub fn branches(&self) -> Vec<TimelineInfo>;
    pub fn head(&self, branch: &str) -> Result<u64, AccessError>;        // Session::sequence (published)
    pub async fn prefix(&self, branch: &str, at_least: u64) -> Result<Prefix, AccessError>;
    pub async fn local_page(&self, branch: &str, after: u64, limit: usize, through: u64)
        -> Result<Page, AccessError>;                                     // Session::local_page
    pub async fn input(&self, branch: &str, input: SourceInput) -> Result<Arc<AgentEvent>, AccessError>;
    pub async fn relay(&self, branch: &str, input: RelayInput) -> Result<Arc<AgentEvent>, AccessError>;
    pub async fn lookup(&self, branch: &str, submission_id: &str) -> Result<Lookup, AccessError>;
    pub async fn published(&self, branch: &str, sequence: u64) -> Result<(), AccessError>;
    pub async fn idle(&self, branch: &str) -> Result<u64, AccessError>;  // Session::idle
    pub async fn stop(&self, branch: &str, reason: String) -> Result<u64, AccessError>; // Session::stop
}
pub struct Prefix { pub through: u64, pub events: Vec<Arc<AgentEvent>> } // ancestor prefix + local
pub struct SourceInput { pub submission_id: Option<String>, pub marker: Marker,
    pub blocks: Vec<SourceBlock>, pub user: Option<UserInput>, pub mode: InputMode }
pub struct Marker { pub kind: String, pub attrs: Vec<(String, String)>, pub body: String, pub key: Option<KeyUse> }
pub struct SourceBlock { pub text: String, pub key: Option<KeyUse> }
pub enum KeyUse { Live(String), Retire(String) }                       // name inside the source namespace
pub struct RelayInput { pub submission_id: Option<String>, pub user: UserInput,
    pub blocks: Vec<SourceBlock>, pub mode: InputMode }
pub enum Lookup { Accepted(Arc<AgentEvent>), Unknown }                  // Ambiguous is an error
```

**Published prefix.** An event is published when its atomic batch is committed to the TimelineStore and the branch publication watermark has advanced ([Timeline storage §3](../../architecture/rust-agent/timeline-storage.md)); readers outside the branch see only published events, and the published head is always a batch boundary. `prefix(branch, at_least)` fixes `through` = the published head at call time, fails with `FutureSequence` if that is below `at_least`, and returns `Session::history(branch, Some(through))` (ancestor prefix plus local events). Reading exactly up to a mid-batch sequence is not offered, because the Timeline rejects it ("inside an atomic batch"). Every event delivered by the broadcast subscription is dispatched, hence published, so `at_least = event.sequence` always succeeds while the branch exists. `stored_page` reads local events of any stored branch without loading (store head when unloaded, precedent `AgentIndex::read_page`); ancestor prefixes require a lease.

**Source marking.** `input` renders `marker` as the first raw block, `context_block::element("dim-<source>", [("kind", kind), attrs..], body)` (A2). Consumers recognize a source only from the first line of the first raw block, `<dim-<source> kind="...">`, never by parsing bodies. Element names already used by existing producers cannot be chosen as source names: `dim-rule`, `dim-state`, `dim-agent-message`, `dim-agent-report`, `dim-branch-task`. Users cannot forge markers: RPC `session.submit` accepts only operator content, not blocks ([prompt](../../architecture/rust-agent/prompt.md)), and `context_block` escapes bodies and attributes. A source name is lowercase `[a-z][a-z0-9-]*`, chosen and owned by the producing plugin (normally its plugin name), stable across releases because historical inputs are recognized by it (A3).

**Keys.** `KeyUse` names are placed in the source namespace: the raw block key is `<source>:<name>` (A4). Prompt's `rule:` and `state:` prefixes can never be produced. A key is per branch Timeline; the same key on another branch is independent.

**Relay.** `relay` writes operator content on the user's behalf at a submission boundary (e.g. converted attachments or content a pre-submission transformation adds) without a marker: the user remains its author (A5). `relay` requires `user`.

**Submission identity.** `submission_id` is generated by the caller per logical submission, `<source>:<uuid>` (A6). Writes do not deduplicate. A caller that may retry after an uncertain outcome (reload, restart, `PublicationUnknown`) calls `lookup` first: `Accepted` returns the committed input, `Unknown` permits a retry, `Ambiguous` (several inputs carry the id) is an error. Same scan as `session.input_lookup` ([connection/tree.rs](../../../crates/dim-app-server/src/connection/tree.rs)).

**Never possible:** writing any event other than `input/message` and `stop`'s `stop/interrupt`; branch-scoped interrupts; editing or deleting history; reading capability state; supplying `submission_id`s, keys or markers of another source (enforced for well-behaved trusted code, not a sandbox).

Errors (`AccessError`):

| Error | From | Meaning |
| --- | --- | --- |
| `SessionClosed` | `AgentError::Closed`, closed slot | Session closed or closing; input not written |
| `SessionUnavailable(reason)` | `AgentError::Storage` from `acquire` | Stored but no available owner, or unreadable |
| `BranchMissing` | `AgentError::UnknownBranch` | Branch never published or already closed |
| `NotLoaded` | `get` | Use `acquire` or `stored_page` |
| `Reloading` | `AgentError::Reloading`, Agents window | Retry; after a write, `lookup` first |
| `Rejected(reason)` | `InputMessageData::validate`, marker/key/source rules, `FutureSequence` | Nothing written |
| `PublicationUnknown` | Committed, publication failed | Outcome uncertain; `lookup` after recovery |
| `Failed(AgentError)` | Fault, storage failure | Session fault; nothing more to do here |

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Write | `SessionLease::input` builds `InputMessageData`, calls `Session::input(data, branch)` → `Branch::input` → `TimelineContext::input`: fault check, close check, `validate()` at the boundary, then `Timeline::append` (no registration callback; `before` hooks may transform the draft and the committed event is final). Returns the committed event; `published` waits on `Branch::publication().watch()` like v3 `session.submit` |
| Consumption | InputQueue's Dispatcher subscription on `input/message` decides applicability by mode and appends `input/applied { mode }`; Loop consumes at a turn or request boundary and appends `input/consumed`; WaitBranch records Steer arrivals (any Steer ends a running `wait`); SessionBroadcast emits it. Handlers register work and return |
| State | InputQueue's `StateCell<InputSnapshot>` and Loop's `LoopSnapshot`, rebuilt from the branch's local history (`local(..=S)`, not the ancestor prefix) on reload |
| Read | `Session::history` / `local_page`, bounded by the published head; no state read |
| Recheck | The caller decides on the prefix it read; it holds no lock. Facts committed after `through` precede its write in sequence and are resolved by the input owners (e.g. a continuation written after an interrupt starts one more turn; overview §5.2) |
| Terminal facts | An input has no terminal fact; its consumption is Loop's. A caller's own work (a job run, a workflow stage) records its outcome in the caller's process storage |
| Reload | `AgentsPlugin` reload reloads `SessionAccess` and all consumers; an input committed before the window stays; a call interrupted by the window returns `Reloading`. InputQueue or Loop reload: the committed input is picked up by the new generation from the Timeline |
| Restart | Leases vanish; committed inputs are restored with the Session and adopted if not yet consumed (existing restore rule); callers look up pending `submission_id`s before retrying, never re-executing their side effects |

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Input before a turn `stop/interrupt`, not yet consumed | applied < interrupt | Stays pending; does not start a turn; consumed with the next turn | Loop |
| Input after `stop/interrupt` | interrupt < input | Normal: Queue/Steer start a turn, Append waits | InputQueue, Loop |
| Input consumed by the interrupted turn | consumed < interrupt | Cancelled with that turn; remains in history | Loop |
| Branch close or branch-scoped interrupt | close started < write | `SessionClosed`/`BranchMissing`; nothing written | Runtime facade |
| Input committed just before branch close | input < branch interrupt | In history, never consumed | Loop |
| Concurrent Steer, Queue, Append from different writers | Timeline order | Each applied by mode in sequence order; no writer sees another's result | InputQueue |
| Tool batch not settled | input < `step/tool-result` | Steer and Append consumed after the batch result; Queue waits for `turn/end` | Loop |
| Steer while the model streams with no output yet | Steer consumed | Request cancelled ("replaced by steer"), same turn continues with the Steer | Loop, ModelProvider |
| Compaction between write and consumption | input < compaction < consumed | Input unaffected; placed at its consumed position after the summary | ContextPreparation |
| Keyed block written while compaction runs | cursor < keyed block | Not in that baseline; delivered at consumption; later baselines replay it | History projection |
| Child branch cut between write and consumption | input < cut < consumed | Child does not adopt the input (its queue is rebuilt from local history) | InputQueue (child) |
| Child branch cut after a keyed block was consumed | consumed < cut | Key in effect in the child; child may supersede or retire it independently | History projection |
| Two writers use the same key | Timeline order | Later block supersedes; namespaces prevent cross-source collisions | History projection |
| Retire then reuse a key | retire < new block | New block starts a new live key | History projection |
| Decision read, then a newer fact, then write | through < fact < write | Write is ordered after the fact; owner consumes in order | Input owners |
| `Reloading` or restart after commit | write committed, reply lost | `lookup(submission_id)` returns `Accepted`; no duplicate | Caller |
| Duplicate submission by a buggy caller | two inputs same id | Both applied; `lookup` returns `Ambiguous` | Caller |
| Session unload between read and write | lease held | Lease pins the instance; unload refused while in use | AgentIndex |
| Session not loaded | — | `acquire` restores (`branch/resume`) before writing | AgentIndex |
| Resource node offline | — | Input committed; turns start when capabilities leave Pending | rutis lifecycle |
| Client detach | — | No effect | — |

## 9. Context mapping

| Input form | What the model sees | Placement |
| --- | --- | --- |
| Marker block | One user-role message with the verbatim element text | First, at the input's consumed position (`input/consumed`) |
| Other raw blocks | One user-role message each, verbatim, in order | After the marker |
| User content (text, images) | One user message; a single text block renders as text, otherwise parts with image blocks | After all blocks of the same input |
| Keyed block (`Live`) | As a raw block; additionally in effect: latest block per key | Consumed position; earlier blocks of the key stay at their positions in uncompacted history |
| Retire block (`Retire`) | Delivered once like a raw block; the key is no longer in effect | Consumed position |

Consumption position by mode: Queue is consumed when Loop starts the turn it drives (after the running turn ends); Steer is consumed at the next boundary of the running turn (after the current batch's `step/tool-result`, or replacing a request that has no output) and starts a turn when idle; Append is consumed with the next model request of a turn and never starts one (an input without `user` and without `mode` is Append). Inputs applied before an interrupt and not yet consumed are consumed by the next turn.

Compaction: consumed inputs survive only through the summary, except keyed blocks, whose latest block of each live key is replayed verbatim from the compaction baseline; retired keys are not replayed. Child branches see the ancestor prefix up to their cut, including consumed inputs and the keys in effect at the cut, and never inputs consumed after it. Non-interactive Sessions use the same mapping; their Queue inputs drive turns with no user present. The model never sees `submission_id`, mode, the source name outside the marker text, the lease, or events of other plugins; reads through this interface have no model-visible effect.

## 10. Errors and recovery

Errors are listed in §6. Cancellation of a caller's future after `Session::input` returned does not withdraw the input. `idle` and `stop` follow the runtime facade: they wait for Loop and return dispatched sequences; `Reloading` during Loop reload resolves when the new generation takes over; a closed branch returns `BranchMissing`. Restoring a Session through `acquire` runs project authorization as AgentIndex does today and can fail with `Failed`.

## 11. Security

Only in-process trusted plugins obtain `SessionAccess`; it is not an RPC method and is not exported over rutis links by this contract. Markers and keys are source-scoped so users and other sources cannot impersonate a source. Writing input confers no authority: tools started by the resulting turn are admitted by Approval and Permission under the Session's policy. Inputs are Timeline facts and are replayed into model context; callers must not put credentials or host security settings into them.

## 12. Acceptance

Flow tests through public interfaces (App Server test suite):

1. `prefix` after a broadcast `turn/end` returns `through` = published head ≥ the event sequence, including ancestor events for a child branch.
2. Queue, Steer and Append written by a source while a tool batch runs: Steer and Append appear after the batch result, Queue after `turn/end`; the marker precedes user content in the rendered request.
3. Interrupt rows: input before an interrupt stays pending and is consumed by the next turn; input after it starts a turn.
4. Keyed block survives compaction as its latest block; a retired key does not; a child branch inherits a live key and retires it without affecting the parent.
5. Marker and key validation: reserved names and foreign namespaces are `Rejected`; RPC cannot submit blocks.
6. `acquire` restores an unloaded Session and writes; `stored_page` reads without loading.
7. A write interrupted by an Agents reload is found by `lookup`; duplicates report `Ambiguous`.
8. Writes to a closed branch return `BranchMissing`/`SessionClosed` and leave no fact.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Attribution is bound per caller through `source(name)` rather than per call | Judgment | Each call names its source; enforcement weaker |
| A2 | Every source input starts with one marker block `<dim-<source> kind="...">` | Overview §5.2, §5.8, §6 ("Input source: raw blocks"); format is judgment | Each plugin defines its own recognition of its inputs |
| A3 | A source name is the producing plugin's own stable name | Judgment | Renaming changes recognition of historical inputs; names must stay stable |
| A4 | Keys are `<source>:<name>` and Prompt's `rule:`/`state:` are reserved | Judgment; existing Prompt key format | Collisions between sources become possible |
| A5 | Submission-boundary content is relayed as the user's without a marker | Overview §3 method E, §5.4, §5.7; judgment | Such inputs gain a marker and render as runtime content |
| A6 | `submission_id` format `<source>:<uuid>`; writes never deduplicate, callers look up | Existing `session.input_lookup`; format is judgment | Write-time deduplication would need registration in the commit lock |
| A7 | Process plugins may call `stop` (turn scope) and `idle` on a Session | Collaboration precedent; overview §5.9 | Callers that stop Sessions they started need another stop path |

## 14. Open questions

| Question | Description |
| --- | --- |
| Overview §13 "How events enter context" | This contract designs for adopted input only (ADR 001) |
| Lease duration | Whether holding a lease should block idle unload by a future host policy, or only pin for the call |
| Ancestor reads of unloaded Sessions | Whether the store should serve full prefixes (with ancestry) without loading |
