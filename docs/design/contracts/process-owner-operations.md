# Process owner operations

Proposed plugin API. Operation types below refer to their owner's existing data records.
A connection-bound client method retains its authentication, caller identity and interruption
policy when its handler moves to the owner.

## Model invocation and accounts

| Catalog entry | Contract |
| --- | --- |
| `modelHost` | invoke(request, mode, consume, cancelled, hooks?) → value. The request is the complete ModelRequest, including pinned tools, config, cached prefix and capability snapshot. |
| `providerConnectionResolver` | Resolution, credential recovery and terminal credential disposition run inside invoke. The native Aimux driver retains its pool and its own unload signal. |
| `providerService` | Consumers call modelHost, modelSelectionHost, selectableModelsHost, auxiliaryModel, modalityTargets or the existing [provider management operations](../appserver/provider-management.md) according to their request. Provider owns record edits and invokes authenticated work locally. |
| `grokOAuthService`, `openAICodexOAuthService` | Status, begin, attempt status, cancel and logout retain each provider's existing management request and reply. Codex also completes its browser callback. Resolve, recover and finish-rejected run at the credential owner. An authenticated driver call executes within the trusted native provider boundary. |

Mode is once or recovering with an optional existing FailureChain. Once is the suggestion
policy. Dialogue uses recovery with the consumer's replay decision. Summary prepares
configuration after resolution and checks its budget before **each** attempt.

- consume(next, cancelled) reads one chunk, end or typed host error at a time. It returns
  {kind: returned, value, replaySafe} or {kind: failed, error, replaySafe}. next expires
  when this attempt's consumer settles. Accepted output prevents unsafe replay.
- hooks.prepare(protocol|null) returns ModelConfig or null once after resolution.
  hooks.beforeAttempt() admits the attempt or returns {rejected: message}.
  hooks.waiting(progress|null) observes the owner's existing retry schedule.
- Cancellation and driver shutdown release a suspended foreign callback. The invocation
  joins accepted work and returns the original result or host error. Malformed wire calls
  return invalid_request. A thrown consumer error cannot manufacture a provider failure.
- Native and foreign bindings select the same service instance. Two independently mounted
  providers must resolve their own driver, credentials and shutdown signal.

## Process control and UI

| Catalog entry | Contract |
| --- | --- |
| `agentIndex` | Session operations use [sessions](../appserver/sessions-service.md). Agent administration and Session creation/fork use their existing management request/reply records. Agent host lookup, active admission guards, releasing a generation and rollback stay with the Agent directory owner. |
| `managedWorktrees` | create({source,base?,branch?}) → CreatedWorktreeReply; discard(created) → void; list({}) → WorktreeView[]; pruneSelection({olderThanMs?,keep?,force}) → PruneSelection; remove({taskId,force}) → boolean. The caller retains rollback on failed Session creation. Paths belong to the worktree node. [Worktree contract](../worktree-management.md). |
| `loader` | Plugin management uses rutis/loader.create, reload, remove and status. Native profile construction and group instantiation stay with the loader owner. A scoped instance operation must preserve the parent and group. |
| `connectionHub` | Presence uses sessionClientsHost.presences; broadcast goes through the connection owner. The transport supplies its authenticated connection identity. Registration, socket writes, close and detach execute against that owner's live connection. |
| `serverControl` | Activity uses processActivity. daemon.ping and daemon.shutdown are served by the process-control owner with the current client protocol; deferred shutdown settles the initiating reply before transport teardown. |
| `uiHost` | ui.open/attach/replace/status/shutdown/close take the existing UI protocol records and return the corresponding client results. The connection owner supplies the caller identity. UI instance generations, attach tokens and disconnect cleanup remain with UiHost. |
| `uiExtensions` | PluginManager uses plugins({}) → string[], refusal({plugin}) → string or null, register({plugin,name,package?}) → void and withdraw({plugin}) → void. Listing, resource reads and change notices use the [UI extension contract](../client-ui-extensions.md). Generation, relative path, revision and connection cleanup retain their original meanings. |

Migrate the model consumers as one invocation family. Ship worktrees, plugin management,
connection control and UI independently. Each family verifies its real native and foreign
caller, including unload during a pending call. Keep external client protocol versions,
owner error codes and credential redaction.
