# Process owner operations

## Retained operations

| Caller → owner | API to use |
| --- | --- |
| Model selection, pickers and auxiliary callers → `providerService` | `resolve`, `selectableModels`, `selectableForModality`, `complete` keep their existing arguments and replies. Provider supplies these operations in the managed profile and ConfiguredModels in the API-key profile. |
| Media selection → Modality | `modalityTargets.resolve`. |
| Management clients → Provider / Grok OAuth / OpenAI Codex OAuth | Existing [Provider management RPCs](../appserver/provider-management.md) and account status, begin, attempt status, cancel and logout RPCs. Codex also handles its browser callback. |
| Session consumers → Agent directory | [`sessions` operations](../appserver/sessions-service.md). Agent administration and Session creation/fork retain the existing `agent.create/list/close` and `session.create/fork` client RPCs. RPC, Agent configuration, Collaboration and Provider lifecycle consumers use directory-owned admission, generation observation, close and rollback. The directory keeps the Session lease through each admitted operation. |
| Plugin management → loader | `rutis/loader.create/reload/remove/status`. The loader owns native profile construction and group instantiation with the original parent and group. |
| Presence consumers → Connection | `sessionClientsHost.presences`. Connection serves this operation directly and handles broadcast, socket registration, writes, close and detach. |
| Activity consumers / daemon clients → ServerControl | `processActivity.activated/hold/release`; existing `daemon.ping/shutdown` RPCs. Shutdown settles the initiating reply before transport teardown. |
| UI clients → UiHost / UiExtensions | Existing `ui.open/attach/replace/status/shutdown/close` RPCs and [extension listing, resources and notices](../client-ui-extensions.md). Transport supplies the authenticated connection identity. UiHost owns attach tokens, generations and disconnect cleanup. |
| Prompt, scenario and summary evaluation tools → active profile | Use `providerService` for their catalog/selection calls and `modelHost.invoke` for execution. |
| Media execution and realtime → native adapter | Existing native connection binding and credential owner ([#1613](https://github.com/arcships/dim/issues/1613), [#1634](https://github.com/arcships/dim/issues/1634)). |
| Transport, Web entry and Session subscriptions → Connection | Connection-owned attach, broadcast, close and detach using the authenticated connection identity. |

## New rutis bindings

| Caller → owner | Methods and replies |
| --- | --- |
| Permission consumers → Agent directory | `sessions.permission({sessionId,branchId})` and `sessions.permissionDefaults({})` retain the existing policy replies. The first holds the Session lease; the second reads the current default without creating a Session. |
| Dialogue, summary and suggestion → active invocation owner | `modelHost.invoke(request, mode, consume, cancelled, hooks?) → value`. Full `ModelRequest` with pinned tools, configuration, cached prefix and capability snapshot. Attempt contract below. |
| Session configuration → `providerService` (managed profile) | `configuration(explicit?, cwd?, preferences) → ModelSelection`; `applyPreferences(current, update) → ModelSelection`. Bind the existing Provider operations. |
| Session creation/adoption and Collaboration → `providerService` (managed profile) | `prepareStartup(request) → {model, admissionId?}`, `checkStartup(source) → void`, `retainStartup({sessionId, model, admissionId?}) → void`, `releaseStartup({admissionId}) → void`. Reuse the pending startup binding ID. Retain on success for that Session generation; release on failure or caller unload. |
| Session creation / worktree management → ManagedWorktrees | `create({source, base?, branch?}) → CreatedWorktreeReply`, `discard(created) → void`, `list({}) → WorktreeView[]`, `pruneSelection({olderThanMs?, keep?, force}) → PruneSelection`, `remove({taskId, force}) → boolean`. Bind the existing operations. Failed Session creation discards the created worktree on its node. |
| PluginManager → UiExtensions | `plugins({}) → string[]`, `refusal({plugin}) → string or null`, `register({plugin, name, package?}) → void`, `withdraw({plugin}) → void`. Bind the existing catalog methods. |

### Model invocation

| Input | Contract |
| --- | --- |
| `mode` | `once` for suggestions; `recovering` accepts an optional existing FailureChain. Dialogue supplies the replay decision. |
| `consume(next, cancelled)` | `next` yields one chunk, end or typed host error. Return `{kind: returned, value, replaySafe}` or `{kind: failed, error, replaySafe}`. The reader expires when the consumer settles. Accepted output prevents unsafe replay. |
| `hooks?` | `prepare(protocol or null) → ModelConfig or null` runs once after resolution. `beforeAttempt() → admitted or {rejected: message}` checks the summary budget before each attempt. `waiting(progress or null)` reports the existing retry schedule. |
| Completion | Cancellation or driver shutdown releases a suspended consumer and joins accepted work. Invalid wire input returns `invalid_request`. Consumer exceptions remain consumer errors. |

Provider (managed profile), ConfiguredModels (API-key profile), or the standalone local model host serves `modelHost.invoke`. The selected owner resolves the request, invokes its driver, and settles credential recovery with the account owner. The local host uses its local connection. The caller receives output or a host error. Aimux owns its native driver, pool and unload signal.
