# Model preferences and Agent configuration

Implementation contract for [F3 #74](https://github.com/arcships/dim/issues/74).
It follows the [design rules](../architecture/rust-agent/principles.md),
[plugin scope](../adr/002-plugin-scope.md) and existing
[Prompt](../architecture/rust-agent/plugins/Prompt.md) contracts.

## Scope and owners

`/effort` changes reasoning effort, `/fast` selects a supported service tier, and
`/agent` adopts an existing Agent profile's prompt and optional model on the current
Session's `main` branch. The profile is a configuration template for this operation.
The Session keeps its Agent owner, identity, workspace, tools, permissions,
capabilities and existing branches. `AgentProfile.tools` is not applied.

| Owner | Responsibility |
| --- | --- |
| Agents, process | Resolve immutable profiles by exact ID from the existing directory; no second mode catalog. |
| Provider, process | Validate advertised effort/tier support and produce typed model configuration. |
| Agents, Session entry | Serve profile adoption through `sessionMethods`; no adopted state or background reconciler. |
| Prompt, branch | Order configuration commands, supersede work, atomically publish facts and rebuild state. |
| PromptSources, Session | Construct the existing `dim/session` System entry from the selected text. |
| CLI | Parse commands, list choices and render replies; no preference authority or shadow configuration. |

The Session entry extends Agents' public entry surface. Its factory captures only
configuration and instance identity; services come from `Deps`. Provider remains a
per-call lookup. Rpc's generic Session-method dispatch does not implement modes.

`AgentsSession` injects process `AgentIndex` and Session `TreeCommands` and
`sessionMethods`, registers the namespace `session.agent` with `sessionMethods`
from its `apply`, and reads the addressed branch's public Prompt service per
call. The Session's method table owns admission, tasks and cleanup of the
registration; unload reports interruption and releases the waiter without
revoking committed Prompt commands. The entry stores no adopted configuration. Its implementation is
[Agents' configuration entry](../../crates/dim-app-server/src/agents/configuration.rs).

## Existing mechanisms

- `session.model.update` already validates `ModelPreferences` through Provider and
  configures a model. Its explicit target starts from catalog defaults, so omitted
  preferences do not preserve previous Session choices.
- `ConfigureUpdate` carries source targets and a model. Prompt owns the existing
  configure-request/result commands, pending records and serial adoption queue.
- `ConfigurationPublisher` publishes prompt/model changes atomically and retains
  the previous tool part.
- `Prompt::open` already preserves System entries and the adopted model on resume
  and child creation. No opening/recovery extension is needed.
- PromptSources' periodic checks already refresh project/user instructions,
  environment and date.
- `Branch::get` reads a capability's published StateCell. The existing
  `PromptSnapshot` contains prompt entries and revisions, but no adopted model
  configuration; F3 adds that derived value to this owner-held snapshot.

## Model parameters

Extend the existing request without adding fields:

```text
session.model.update {
  sessionId, branchId, providerId?, modelId?,
  modelPreferences?: {reasoningEffort?, serviceTier?}
} -> {model: {providerId, modelId}, outcome: "adopted" | "unchanged"}
```

Without a target, at least one preference is required; an empty update is invalid.
Both target fields are present or both absent; partial targets and explicit null
are invalid. Explicit-target requests keep their current semantics. Without a
target, read the current adopted configuration from Prompt's branch snapshot and apply
only supplied preferences to that captured base. Other configuration values remain
unchanged. Provider owns validation and native reasoning-off/tier mapping; the CLI
does not reconstruct complete preferences from provider-specific request fields.
Changing effort replaces mutually exclusive reasoning enable/budget controls;
unrelated sampling, token limits, service tier and metadata are preserved.

This extends the business request's choice of base configuration. `ConfigureUpdate`,
`ModelSelection`, Timeline events and Session facades remain unchanged. No change
returns `unchanged` without a new adopted configuration fact. Requests still support `main` only. An older
daemon rejects the new request form normally; the client does not retry with a
guessed complete selection.

`/effort VALUE`, alias `/reasoning VALUE`, sends `reasoningEffort` without a target.
Values come from declared `effortOptions` and the existing reasoning-off declaration.
The CLI reads the adopted provider/model identity from the public
`context/configured` view, then reads choices from the existing `provider.catalog`
response. The public event does not expose internal model settings or capability
snapshots. Catalog choices are display data; Provider validates each update.
`none` needs that declaration or an advertised native `none`. Unsupported values
fail without adoption; no value is guessed from a model name.

`/fast on` sends `serviceTier: "priority"`; `/fast off` sends
`serviceTier: "default"`. The shortcut requires both advertised values and a
protocol supporting this preference. Other tier names do not implicitly mean fast
mode. Effort, token limits and other settings from the captured base are retained.
There is no persisted `fast` flag.

The snapshot field uses the existing `ModelConfiguration` type. Prompt's
projection updates it from adopted `context/configured` facts, and initializes it
from the existing context/checkpoint reconstruction when a branch opens or
restores. `Branch::get::<PromptSnapshot>()` supplies the latest published value;
a missing or reloading state follows the existing unavailable/error behavior.
An available snapshot without adopted model configuration rejects an absent-target
request with the existing `invalid_params` error; the caller can supply an explicit
provider/model target. This is not a service reload.
Preference commands do not call `Branch::context`, replay history or fill missing
configuration with defaults. This adds no service key, runtime state, persisted
field or second configuration type.

The [Provider management proposal](appserver/provider-management.md#33-probe-preference-adoption-and-usage)
adds persisted per-model `ModelPreferences`, superseding F3's no-persisted-preference
scope. Provider owns that persistence; clients read it and explicitly pass chosen
preferences through the existing Session adoption methods. The server never
automatically applies them. Clients must include the exact `{providerId, modelId}`
with saved preferences for that pair; an unavailable target fails without retrying
with an implicit selection. `/fast` still changes only the current Session. A saved
`serviceTier` is a persisted fast/tier setting applied only when a client passes it;
there is still no persisted `fast` boolean. This addition is a proposal, not an
implemented part of this F3 Session path.

## Agent configuration adoption

Add one mutating Session method, contributed by Agents in namespace `session.agent`:

```text
session.agent.update {sessionId, branchId, agentId}
  -> {outcome: "adopted" | "unchanged"}
```

It uses existing `sessionMethods` admission/cancellation and supports `main` only.
Agents resolves a currently available profile by exact ID. Its `systemPrompt`
replaces the `dim/session` entry. An absent profile model preserves the adopted
model and preferences. A specified model adopts the resolved target's defaults,
including when it names the current model. Names/descriptions remain UI data.
No tool list, permission or Session ownership record is written.

PromptSources constructs the entry with its current identity, main branch scope,
System delivery, order and content hash. Empty prompt text revokes the entry. Base
instructions, Skills, AGENTS.md and environment sources keep their own entries.

Prompt's existing branch service gains a resolved-source operation. Its data is an
optional `ModelSelection` and a list of `{target: PromptSourceTarget, entry:
PromptEntry | null}`; null revokes that target. The Rust binding reuses these types
and Prompt's current loaded-source representation, derives the configure targets
and freezes the values in the existing operation's pending record. Each non-null entry
must match its target source/scope; System entries require branch scope, matching
the ordinary source-loading contract. It writes the
existing configure command, enters the same Dispatcher/queue and publishes through
the existing result path. The caller never writes the adopted result directly.

The service keeps the existing branch `Prompt` key and branch instance identity.
`configureLoaded` is asynchronous and returns the existing configuration outcome;
dropping its waiter does not revoke a committed command. Prompt owns cancellation
on supersession/unload. This adds no second service key or named-service adapter;
cross-language exposure remains C4's work, not an assumed F3 prerequisite.

This is Prompt configuration behavior, not runtime infrastructure. Neither
`ConfigureControl` nor `ConfigureUpdate` changes. Source-name-only loading cannot
carry selected profile text without mutating a shared source behind another
request's back; frozen values avoid that staging state and additional locks.

```text
/agent ID -> existing sessionMethods dispatch
          -> Agents resolves profile; Provider resolves optional model
          -> Prompt registers frozen values and writes configure-request
          -> existing Dispatcher, adoption queue and atomic publication
          -> Prompt consumes facts; reply adopted/unchanged
          -> later requests use the configuration and existing tools
```

PromptSources' explicit refresh uses its existing dynamic `CHECKED` sources,
matching periodic refresh. It does not reload static base/Session sources and so
cannot restore the creation-time prompt after a switch. Opening already preserves
System entries. Restored Sessions retain adopted prompt/model facts; new branches
inherit their cut, and existing branches do not follow later main changes.

## Ordering and failure

Already prepared requests and their tool handlers are unchanged. Later preparations
use adopted facts. Resolution/validation failure leaves configuration intact.
Prompt retains its existing supersession, cleanup and unchanged outcomes;
supersession uses the existing configuration error behavior. A lost response is
not automatically retried because adoption may have committed. Closing a profile
after copying its values does not revoke adopted content. Provider credentials are
not copied into prompts or persisted configuration facts.

One CLI awaits each command before sending another command/input. Across clients,
a preference candidate can be prepared from a prefix preceding another client's
change; Prompt ordering can then adopt that candidate's older base model/settings.
This is a known limit, not a concurrent patch guarantee. Record it for consolidated
configuration design. Moving the read into Prompt's queue would also require
validating the delta against whichever model that queue then contains. The
current Provider operation asynchronously resolves catalog/protocol metadata in
the App Server and returns a complete typed selection; Prompt's queue accepts
that selection, not Provider's business preference type or a resolver callback.
Adding a Provider dependency or a new deferred validation/patch contract is not
part of this design. Keep the captured-candidate semantics without revisions,
CAS, retries, a global lock or another serialization layer.

Errors reuse existing invalid-parameter, Agent-unknown, model/provider,
configuration-superseded, reload/interruption and Session-closed contracts.

## Scrollback interaction

Commands run at the existing idle prompt. Bare `/effort` and `/fast` read the
adopted provider/model identity from public `context/configured` history/notifications
and show supported choices from the existing `provider.catalog` response, without
mutation or a new query method. Bare `/agent` uses existing `agent.list` IDs, names
and descriptions; selection uses an exact ID. Empty/unavailable choices are explicit.

```text
> /effort high
Reasoning effort configuration adopted: high
> /fast off
Service tier configuration adopted: off
> /agent
ID                 NAME              DESCRIPTION
<agent-id>         Reviewer          Review changes
> /agent <agent-id>
Agent prompt/model configuration adopted; Session tools unchanged.
> /fast on
Error: this model does not advertise priority/default service tiers.
> _
```

Enter submits a command; errors keep the prompt and Session. Ctrl+C/EOF keep the
existing behavior for turns. Ctrl+C during configuration waits for its accepted
operation's result before another line; the reply may describe an already adopted
change. Further interrupts keep waiting and are consumed before returning to the
editor rather than clearing or exiting the next draft. Lines wrap at terminal width and use existing escaping. No
overlay, fullscreen UI, input queue or typeahead change. Command text does not
become model input; headless prompt/stdin content keeps its literal meaning.

## Impact and verification

| Area | Necessary change |
| --- | --- |
| Provider / current model-update entry | Apply preferences to an adopted base when no target is supplied; preserve explicit-target behavior. |
| Agents Session entry | Resolve a profile and submit prompt/model through one contributed method. |
| Prompt | Accept frozen sources through its current command/adoption path; expose adopted ModelConfiguration in its existing published snapshot. |
| PromptSources | Reuse System-entry construction and refresh dynamic sources only. |
| CLI | Three commands, choice display and replies. |

New public content is the absent-target model-update semantics, the Agent Session
method, Prompt's resolved-source operation and its derived configuration snapshot
field. Each serves the operations above.
No core port, durable event, persistence format, loader option, lifecycle stage,
dependency manager or general configuration registry is added.

Reuse public flows: actual Provider HTTP requests check effort/off/tier mapping and
untouched settings; transport checks both model-update forms and rejection without
adoption; Prompt lifecycle checks atomic adoption, supersession, failure, unchanged
and restoration; CLI binary/PTY flows check routing and recovery. Include absent
and present profile models, empty prompts, unchanged tools/permissions/owner,
refresh after switching, child cut inheritance and an already bound request.
Verify that preference reads use the published snapshot after adoption and
restore, without history replay on each command; concurrent clients retain the
documented complete-candidate ordering rather than a delta-merge guarantee.
Do not test private queue/helper calls. Run graph/check when implementing the
Session entry registration, not for this design document.

Ablation: existing profiles replace a mode store; facts replace a preferences cache;
existing model-update fields replace a separate parameter method; frozen values
replace mutable staging; existing Prompt ordering/opening replaces new lifecycle
logic. The Session entry serves the user operation and the resolved-source path
supplies immutable content. Neither owns another module's state.

## Rule check

| Rule | Application |
| --- | --- |
| §4.1 | Existing Prompt commands request changes; Prompt writes adopted facts. |
| §4.2 | Prepared requests/bindings retain handed-over configuration. |
| §4.3 | No before callback or entry-side adoption; entries check shape and scope. |
| §4.4 | No core business branch; Provider owns mapping and Prompt owns adoption. |
| §4.5 | ConfigurationPublisher commits prompt/model facts atomically. |
| §4.6 | Context follows existing adopted configuration and consumed inputs/results. |
| §4.7 | Public profiles, model resolution, published Prompt state and Prompt operations. |
| §4.8 | Existing infrastructure suffices; no mechanism-owner requirement. |
| §4.9 | Only adopted facts are durable; names, choices and replies remain UI data. |
| §1 / D1 | Process catalogs, Session entry and branch state keep their owners; App Server adds only the Agents entry. |
| D2 | No registry/manager wiring part; the entry serves profile adoption. |
| D3 | Dependencies point outward; Provider lookup per call does not gate core loading. |
| D4 | Business changes adopt data without loader edits/reloads. |
| D5 | Client methods and the resolved-source operation have data-only contracts; Rust uses existing types. No new cross-language framework contract is assumed. |

## Requirements raised to mechanism owners

None. Timeline, Dispatcher, envelopes, assembly/lifecycle and Session/branch
facades remain unchanged. The known concurrent-candidate limit is not a request
for new infrastructure.
