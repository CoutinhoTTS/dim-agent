# Recall

The Recall plugin registers Recall in every Session: the `recall` tool tells
the model the history of its own branch. Design:
[Recall](../../../design/plugins/Recall.md).

## Recall

Owner: [recall.rs](../../../../crates/dim-agent-capabilities/src/recall.rs).
Registers the Session `recall` tool. A call resolves the calling branch from
`ToolExecutionContext.branch_id`, pins its published head with
`Branch::sequence()`, reads `Branch::history(Some(head))` once, and answers
from that list: the turn index, one turn, one thing in full, or a search.

### Dependencies

| Service | Purpose |
| --- | --- |
| `S:TreeCommands` | Resolve the calling branch and read its pinned history per call. |
| `S:ToolRegistry` | Register `recall`, until this part unloads. |

### Events and lifetime

Reads history; writes no Timeline event, holds no `StateCell`, subscribes to
nothing and starts no task. Each call folds the pinned events into one origin
event per thing and the later events a join id attaches to it, and tells only
the kinds the design lists; every other kind, core or opaque, is skipped
without decoding its data, so a new kind needs no change here. Counting
follows context assembly: an input counts once it is consumed, a reply counts
when its output completed or as the partial text an interruption left, and a
tool result is told as its `content`.

The pinned read is the whole call: a compaction during the call cannot move
it. A facade read while the Session reloads returns
`HostError::Cancelled("capability reloading")`, any other facade error is a
failed call, and the call's cancellation token is checked after the read and
before rendering, as `wait` does. Pages are 20 entries, or 8,000 characters of
one thing in full; every variable part of a line is cut at 120 characters, so
a line stays at most 600 characters. The tool returns text the model reads and
`structuredContent` for UI and persistence only.

### Tests

[recall_flow.rs](../../../../crates/dim-agent-capabilities/tests/recall_flow.rs)
holds the acceptance flows of the design's §14: the turn index and its pages,
a turn, one thing in full with CJK paging, the allowlist against what context
assembly places, search, compactions, call endings, branch and Session forks,
line bounds, the read-only preset, a long turn, interrupted outputs and the
argument errors.
[transport_flow/recall.rs](../../../../crates/dim-app-server/tests/transport_flow/recall.rs)
finds `recall` in the tool table of both the product and the `--models`
profile.

<!-- generated:plugin Recall — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:ToolRegistry`、`S:TreeCommands` |
| 提供 | — |
| 被注入于 | — |
<!-- /generated -->
