# Process owner operations

## Existing APIs

| Caller → owner | API to use |
| --- | --- |
| Model selection, pickers and media consumers → Provider / Modality | `modelSelectionHost.resolve`, `selectableModelsHost.selectableModels/selectableForModality`, `auxiliaryModel.complete`, `modalityTargets.resolve`. |
| Management clients → Provider / Grok OAuth / OpenAI Codex OAuth | Existing [Provider management RPCs](../appserver/provider-management.md) and account status, begin, attempt status, cancel and logout RPCs. Codex also handles its browser callback. |
| Session consumers → Agent directory | [`sessions` operations](../appserver/sessions-service.md). Agent administration and Session creation/fork retain the existing `agent.create/list/close` and `session.create/fork` client RPCs. The directory owns admission, generation release and rollback. |
| Plugin management → loader | `rutis/loader.create/reload/remove/status`. The loader owns native profile construction and group instantiation with the original parent and group. |
| Presence consumers → Connection | `sessionClientsHost.presences`. The connection owner handles broadcast, socket registration, writes, close and detach. |
| Activity consumers / daemon clients → ServerControl | `processActivity.activated/hold/release`; existing `daemon.ping/shutdown` RPCs. Shutdown settles the initiating reply before transport teardown. |
| UI clients → UiHost / UiExtensions | Existing `ui.open/attach/replace/status/shutdown/close` RPCs and [extension listing, resources and notices](../client-ui-extensions.md). Transport supplies the authenticated connection identity. UiHost owns attach tokens, generations and disconnect cleanup. |

## New rutis bindings

| Caller → owner | Methods and replies |
| --- | --- |
| Dialogue, summary and suggestion → model host | `modelHost.invoke(request, mode, consume, cancelled, hooks?) → value`. Full `ModelRequest` with pinned tools, configuration, cached prefix and capability snapshot. Attempt contract below. |
| Session configuration → Provider | `configuration(explicit?, cwd?, preferences) → ModelSelection`; `applyPreferences(current, update) → ModelSelection`. Bind the existing Provider operations. |
| Session creation/adoption and Collaboration → Provider | `prepareStartup(request) → {model, admissionId?}`, `checkStartup(source) → void`, `retainStartup({sessionId, model, admissionId?}) → void`, `releaseStartup({admissionId}) → void`. Reuse the pending startup binding ID. Retain on success for that Session generation; release on failure or caller unload. |
| Session creation / worktree management → ManagedWorktrees | `create({source, base?, branch?}) → CreatedWorktreeReply`, `discard(created) → void`, `list({}) → WorktreeView[]`, `pruneSelection({olderThanMs?, keep?, force}) → PruneSelection`, `remove({taskId, force}) → boolean`. Bind the existing operations. Failed Session creation discards the created worktree on its node. |
| PluginManager → UiExtensions | `plugins({}) → string[]`, `refusal({plugin}) → string or null`, `register({plugin, name, package?}) → void`, `withdraw({plugin}) → void`. Bind the existing catalog methods. |

### Model invocation

| Input | Contract |
| --- | --- |
| `mode` | `once` for suggestions; `recovering` accepts an optional existing FailureChain. Dialogue supplies the replay decision. |
| `consume(next, cancelled)` | `next` yields one chunk, end or typed host error. Return `{kind: returned, value, replaySafe}` or `{kind: failed, error, replaySafe}`. The reader expires when the consumer settles. Accepted output prevents unsafe replay. |
| `hooks?` | `prepare(protocol or null) → ModelConfig or null` runs once after resolution. `beforeAttempt() → admitted or {rejected: message}` checks the summary budget before each attempt. `waiting(progress or null)` reports the existing retry schedule. |
| Completion | Cancellation or driver shutdown releases a suspended consumer and joins accepted work. Invalid wire input returns `invalid_request`. Consumer exceptions remain consumer errors. |

Each call uses its selected Provider and driver. Provider resolves the connection and coordinates credential recovery and terminal disposition with the account owner. Model callers receive output or a host error. The native Aimux driver owns its connection pool and unload signal.
