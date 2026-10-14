# Notifications

Status: Draft. Overall design: [overview §5.11](../overview.md#511-notifications). Phase: P2. Language: TypeScript first ([overview §11.3](../overview.md#113-assignment)).

Notifications delivers messages to configured external channels (Slack, Feishu) when a task completes or fails, an approval is pending or a question awaits an answer, and lets the model send a message through the process-level `notify` tool. It observes Sessions only through the [broadcast subscription](../contracts/session-broadcast-subscription.md) and public event data contracts, confirms each trigger against a fixed prefix read through [session-access](../contracts/session-access.md), and writes nothing to any Timeline.

## 1. User flows

1. The user configures a Slack channel (incoming webhook stored as a credential reference) with triggers `turn.completed` and `approval.pending`.
2. A long task on `main` ends normally and nothing else is pending. After the settle delay, Notifications reads the prefix, confirms the branch has no newer turn and no pending driving input, and posts "Task completed" with the Session reference.
3. A tool call waits for approval. If it is still pending after the pending delay, the channel receives "Approval needed: <tool name>". An approval answered in time sends nothing.
4. The model calls `notify {message, channel?}`; after admission under "outbound message", the message is posted and the result says where it went.
5. Delivery fails (network, 5xx); Notifications retries with backoff, then records the failure in diagnostics. The Session is unaffected.
6. In-terminal reminders (bell, focus) are the CLI's, not this plugin's (overview §5.11).

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Trigger evaluation from public events and prefix facts | Meaning of other plugins' facts beyond their public data contracts |
| Channel adapters (Slack, Feishu), message formatting, retries, rate limits | In-terminal reminders and desktop OS notifications (CLI) |
| `notify` tool | Approval or answering through a channel (no inbound messages) |
| Channel configuration schema under `notifications.*` | Storage of credentials (secret store) and of configuration (Settings) |
| Delivery records in diagnostics | Scheduler job state (§14) |

## 3. Scope and dependencies

**Contracts depended on:** [`SessionEvents`](../contracts/session-broadcast-subscription.md), [`SessionAccess`](../contracts/session-access.md) (read-only: `get`, `prefix`, `stored_page`; never `input`), [`ProcessToolSource`](../contracts/process-tool-contribution.md), the public data contracts of `turn/start`, `turn/end`, `input/message`, `input/consumed`, `step/text`, `approval/request`, `approval/respond`, `approval/cancelled` (`crates/dim-agent-types/src/events.rs`) and `question/asked`, `question/answered`, `question/cancelled` (declared by the owner of overview §5.1; consumed only when registered), the Settings typed configuration for the loader editable layer (overview §7.2), a credential-reference resolver (§14), the Permission risk category `outbound_message` (overview §7.1), the Management contribution contract (overview §5.16), and optionally (`When`) SessionStore title metadata (overview §7.2) for message labels.

**Contracts defined here:** the `notify` tool, trigger names (`turn.completed`, `turn.failed`, `approval.pending`, `question.pending`), the configuration document `notifications`, and management method `notifications.test`.

| Part | Scope | Injects | Provides | Status |
| --- | --- | --- | --- | --- |
| `Notifications` | Process (App Server) | `Arc<SessionEvents>`, `Arc<SessionAccess>`, Settings configuration, credential resolver, `When<Arc<SessionMetadataStore>>` | `ProcessToolSource` named `notifications`; Management contribution `notifications.test` | Proposed |
| Permission | Branch | existing | category `outbound_message` | Supplement (overview §7.1) |
| Settings | Process | existing | `notifications` configuration document | Supplement (overview §7.2) |

Delivery runs on the control node (overview §3.1): it uses the control node's network, not Session-bound resource services.

## 4. Events

None (overview §5.11). Notifications decides nothing that a branch consumes later. Delivery attempts, outcomes and drops are diagnostics records `{channel, trigger, session, branch, sequence, outcome, attempts}` without message bodies. A `notify` call and its outcome are the existing `step/tool-call` and `step/tool-result`.

Events consumed:

| Event | Role | Rule |
| --- | --- | --- |
| `turn/end` (notice, `branch = main`) | Trigger | `completed` → `turn.completed`; `failed` → `turn.failed`; `cancelled` → none (A3) |
| `approval/request` (notice, any branch) | Trigger | `approval.pending` if no `approval/respond` or `approval/cancelled` with the same `approval_id` in the prefix read after the delay |
| `question/asked` (notice, any branch) | Trigger | `question.pending` if no `question/answered` or `question/cancelled` with the same `call_id` in the prefix read after the delay |
| `turn/start`, `turn/end`, `input/message`, `input/consumed` (prefix) | Check | Turn triggers fire only when `N` is the latest `turn/end`, no turn is open after it and no Queue/Steer input is unconsumed |
| `step/text` (prefix) | Content | Optional excerpt of the turn's last request (`includeContent`) |

## 5. State

| State | Owner | Rebuild |
| --- | --- | --- |
| Channel configuration | Settings (loader editable layer) | Persistent; watched for changes |
| Delayed checks and deliveries in flight | Notifications, memory | Not rebuilt; cancelled on unload |
| Dedup set `(session, branch, trigger, id)` for this generation | Notifications, memory | Not rebuilt (no replay exists, so no duplicates across generations) |
| Rate-limit windows per channel | Notifications, memory | Not rebuilt |

No state is derived from Timeline into owned state; every trigger is checked against a fresh prefix.

## 6. Interfaces

### Configuration (`notifications` document, user scope only, A8)

```text
notifications.channels: [{
  name: string,                       // [a-z0-9-]{1,32}, unique
  kind: "slack" | "feishu",
  target: { slack: { webhookRef } | { botTokenRef, channelId } }
        | { feishu: { webhookRef, signingSecretRef? } },
  triggers: ("turn.completed" | "turn.failed" | "approval.pending" | "question.pending")[],
  includeContent: bool,               // default false (A5)
  allowTool: bool,                    // default true; channel offered to `notify`
  enabled: bool                       // default true
}]
notifications.settleDelaySeconds: u32   // default 3, 0..=60 (A2)
notifications.pendingDelaySeconds: u32  // default 10, 0..=3600 (A2)
notifications.defaultChannel: string?   // used by `notify` without `channel`
```

`*Ref` fields are credential references; webhook URLs are treated as credentials. Configuration is changed only through typed Settings operations; Notifications reads it and reacts to changes.

### Tool `notify`

| Item | Value |
| --- | --- |
| Parameters | `{message: string (1..=4000), channel?: string}`; `channel` is an enum of channels with `allowTool && enabled` (A6) |
| Result | `ToolResult.content`: `Notification sent to <channel>.`; errors: `Unknown channel <name>.`, `No notification channel is configured.`, `Delivery failed: <fixed reason>.`, `Rate limit reached for <channel>.`, `Cancelled; delivery may have occurred.` |
| Risk category | `outbound_message` (overview §5.11) |
| Requirements | `interactive: false`, `roles: None` |
| Set | key `notifications`; offered only when at least one channel allows the tool; version changes with the channel list |

Messages sent by the tool are prefixed `[dim <session label>] ` (A7).

### Management method

| Method | Params | Result |
| --- | --- | --- |
| `notifications.test` | `{channel}` | `{ok, error?}`; sends a fixed test text (A9) |

No Session methods, no CLI command beyond Settings and `notifications.test`.

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Subscribe | In `apply`: one subscription `{types: {"turn/end", "approval/request", "question/asked"}, sessions: None, branches: None}`, capacity 256; `turn/end` notices from branches other than `main` are dropped by Notifications (A1). Types whose owner is not registered simply never arrive |
| Trigger | For a notice matching an enabled channel's trigger and not in the dedup set: register a delayed check (settle delay for turn triggers, pending delay for pending triggers) with the plugin's task tracker; return to `next()` |
| Check | After the delay: `get(session)` → `prefix(branch, at_least = sequence)`; if not loaded, `stored_page(session, branch, after = sequence - 1, ...)` for local facts (A4). Apply the rule of §4 to the facts read. Wall-clock only delays the read; the decision is by facts and sequence |
| Format | Fixed templates (§9); optional content excerpt; label = metadata title when available, otherwise the Session id (A7) |
| Deliver | Resolve credential references at send time; HTTPS POST; retry on network error, 429 and 5xx with 1 s, 4 s, 16 s backoff (3 attempts); no retry on other 4xx (A10). Rate limit 20 messages per minute per channel; excess dropped with a diagnostic (A10) |
| Timeline writes | None. No Dispatcher registration, no `StateCell` |
| Tool call | ToolExecutor admits via Approval/Permission (`outbound_message`), then calls the handler with `ProcessToolSession` and a `CancellationToken`; the handler delivers synchronously and returns. Settlement belongs to ToolExecutor |
| Recheck | The prefix read after the delay is the recheck for event triggers; nothing else is written |
| Reload / restart | Delayed checks and deliveries are cancelled through `ctx.effect`; a request already sent may or may not have been delivered and is never resent by the new generation (time bus §5). Notices missed while down are not replayed; pending approvals at restart are settled by Approval's recovery and need no notice |

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Turn ends, continuation or queued input follows before the check | `turn/end` N < input | Pending driving input or newer turn: no `turn.completed` | Notifications |
| Turn ends, input arrives after the check | check < input | Notification sent; work continues (accepted) | Notifications |
| Turn ended by interrupt | `turn/end {cancelled}` | No notification (user-initiated) (A3) | Notifications |
| Turn failed | `turn/end {failed}` | `turn.failed` if still the latest turn | Notifications |
| Approval answered before the delay | request < respond < check | Nothing sent | Notifications |
| Approval cancelled by interrupt or branch close | request < `approval/cancelled` | Nothing sent | Notifications |
| Approval answered after send | check < respond | Message stays sent; no follow-up (A11) | — |
| Question answered or cancelled before the delay | asked < answered/cancelled | Nothing sent | Notifications |
| Question events not registered (owner absent) | — | Trigger inert | — |
| Concurrent Steer/Queue/Append | Timeline order | Queue/Steer unconsumed at check suppresses `turn.completed`; Append does not | Notifications |
| Tool batch not settled | approvals inside the batch | Pending approval notified; `turn/end` cannot precede the batch result | Notifications; Loop |
| Compaction | — | No effect: checks read facts | — |
| Child branch | approval in child | Notified (any branch); child `turn/end` ignored (A1) | Notifications |
| Branch close during the delay | — | `BranchMissing`: nothing sent | Notifications |
| Session unloaded during the delay | — | `stored_page` reads local facts; decision as usual (A4) | Notifications |
| Session closed or deleted | — | `SessionClosed`/not found: nothing sent | Notifications |
| Duplicate notice, resubscribe | — | Dedup set within the generation | Notifications |
| Gap notice | — | Missed triggers are not recovered (best effort) (A12) | Notifications |
| Interrupt during `notify` before sending | cancel < send | No request; result `Cancelled` | Notifications; ToolExecutor |
| Interrupt during `notify` after sending | send < cancel | Request aborted; result `Cancelled; delivery may have occurred.` | Notifications |
| Notifications reload or restart mid-delivery | — | Cancelled; never resent; in-flight `notify` batches settled as reload by ToolExecutor; restart settles committed calls without re-execution | ToolExecutor; Notifications |
| Configuration changes during the delay | — | Channel set and triggers read at check time | Notifications |
| Credential reference unresolvable | — | Delivery fails; diagnostic | Notifications |
| Resource node offline | — | No effect (control node delivers); the Session's events may pause | — |
| Client detach | — | No effect; notifications are sent regardless of attachment (A13) | — |

## 9. Context mapping

Event-triggered notifications have no model-visible effect: nothing is written to any Session, and channel messages never return to the model.

| What the model sees | Form and position |
| --- | --- |
| `notify` declaration | Name, description `Send a short message to one of the user's configured notification channels.`, schema with the channel enum, in the adopted `context/configured`; only while at least one channel allows the tool |
| `notify` outcome | `ToolResult.content` (§6) at the tool-result position of its batch; survives compaction only through the summary; child branches inherit results before their cut |
| Non-interactive Sessions | Same declaration and results |
| Never visible | Channel targets, credential references, delivery records, retry state, other Sessions' triggers, diagnostics, other plugins' events |

Message templates (text sent to channels, not to the model): `turn.completed`: `[dim <label>] Task completed.`; `turn.failed`: `[dim <label>] Task failed.`; `approval.pending`: `[dim <label>] Approval needed: <tool name>.`; `question.pending`: `[dim <label>] The agent is waiting for your answer.` With `includeContent`: turn triggers append up to 500 characters of the last request's `step/text`; `approval.pending` appends the request `reason`; `question.pending` appends the question texts up to 500 characters (A5). Approval inputs are never sent.

## 10. Errors and recovery

| Case | Result |
| --- | --- |
| Invalid configuration | Rejected by Settings validation at the write boundary; Notifications keeps the last valid document |
| Delivery failure after retries | Diagnostic; no Session effect; tool returns an error result |
| Cancellation (interrupt, reload) | Outstanding requests aborted; resources released before the tool returns |
| `notifications.test` | Returns fixed error codes `unknown_channel`, `credential_unavailable`, `delivery_failed`, `rate_limited` |
| Process exit | In-memory checks lost; no persistent queue (A12) |

## 11. Security

`notify` is admitted per call under `outbound_message`; under the default presets it asks (FullAccess behavior is overview §13). The model can choose only among user-configured channels, never a URL. Channels come only from the user scope of Settings, not from project files or plugins (A8), so a repository cannot define an exfiltration target. Credentials are references resolved at send time and never logged, returned to the model or placed in diagnostics. Event-triggered messages carry no Session content unless `includeContent` is enabled per channel. Feishu signing uses the referenced secret when configured. Subscribers see all Sessions' events (subscription contract §11); Notifications forwards only the template fields above.

## 12. Acceptance

Flow tests with fake HTTP endpoints and a fake model, through public interfaces:

1. Completed `main` turn with nothing pending sends one `turn.completed`; a Queue input committed before the check suppresses it; a cancelled turn sends nothing; a failed turn sends `turn.failed`.
2. Approval answered before the pending delay sends nothing; unanswered sends once; cancelled by interrupt sends nothing.
3. `question/asked` with and without an answer before the delay (when Questions is registered); inert otherwise.
4. `notify` without channels is not declared; with one channel it sends and returns the result; Permission asks under `outbound_message`; interrupt before and during delivery.
5. Retry on 5xx, no retry on 400, rate limit drop, credential failure.
6. Reload during a delivery: no resend after reload; restart settles a committed `notify` call without re-execution.
7. `includeContent: false` messages contain no Session text; diagnostics contain no bodies or URLs.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Turn triggers only on `main`; approval and question triggers on any branch | Judgment | Child-branch work would notify on every sub-task |
| A2 | Settle delay 3 s and pending delay 10 s defaults | Judgment | Earlier or later messages |
| A3 | Interrupted turns do not notify | Judgment (the user stopped it) | Interrupts notify too |
| A4 | Unloaded Sessions are checked through `stored_page` | Judgment | Triggers for unloaded Sessions are dropped |
| A5 | Event messages carry no Session content unless `includeContent`; excerpt limit 500 characters | Judgment (overview §9 sensitive data) | Content always or never included |
| A6 | `notify` parameters and channel enum | Judgment | Different schema |
| A7 | Messages are labeled with the metadata title if available, else Session id | Judgment | Other labels |
| A8 | Channels are configurable only in user scope | Judgment from overview §9 Trust | Project channels would need host-local approval |
| A9 | `notifications.test` management method | Judgment | Users test by triggering real events |
| A10 | Retry 3 attempts (1 s/4 s/16 s) and 20 messages per minute per channel | Judgment | Other policies |
| A11 | No follow-up message when a pending item resolves after notification | Judgment | Extra resolved messages |
| A12 | Delivery is best effort: no persistent queue, no recovery after Gap or restart | Overview §5.11 (no events); judgment | A process-storage outbox is needed |
| A13 | Notifications are sent regardless of attached clients | Judgment (detach is not close) | Suppression requires an attachment contract |
| A14 | Slack via incoming webhook or bot token + channel id; Feishu via custom-bot webhook with optional signing | Feature list "Slack and Feishu"; modes are judgment | Other adapters |

## 14. Open questions

| Question | Description |
| --- | --- |
| Scheduled job end | Overview §5.11 lists "a scheduled job ends". Job Sessions produce `turn/end`, which covers it as `turn.completed`/`turn.failed`; a job-level notice would need a public contract from the scheduler's owner rather than interpretation of its internals |
| Credential resolution | The existing resolver (`ResolveProviderSecret`) is Provider-specific; a generic credential-reference resolver should be a Settings supplement |
| Overview §13 "FullAccess and new risk categories" | Whether `outbound_message` asks under FullAccess |
| Overview §13 "Process-level subscription" | Depends on the subscription contract's answer |
| Inbound actions | Answering approvals or questions from Slack/Feishu is out of scope; it would need RemoteGateway-level authentication |
