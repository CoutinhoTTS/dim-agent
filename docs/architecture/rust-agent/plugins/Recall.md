# Recall

The Recall plugin registers Recall in every Session: the `recall` tool tells
the model the history of its own branch. Design:
[Recall](../../../design/plugins/Recall.md).

## Recall

Owner: [recall.rs](../../../../crates/dim-agent-capabilities/src/recall.rs).
Registers the Session `recall` tool. A call resolves the calling branch from
`ToolExecutionContext.branch_id` and reads that history backwards through
`Branch::query`: the chain scope, descending, the kinds the narrative folds, one
page of events per call, continued until the answer is met. A call reads the
published head of its branch once, before the first page, and starts every page
at that sequence, so the selection is fixed for the whole call: a fact published
while the read runs is outside it, and the offsets stay on one contiguous suffix.
It does not fold while it pages: it stops when the collected pages reach the turn
or the count the answer needs and folds them once; a search whose fold finds
fewer matches than a page finishes the read and folds the whole chain once more,
so a call folds at most twice. It answers from the collected suffix: the
turn index, one turn, one thing in full, or a search. The turn a page cuts is
not told; everything after it is folded as the whole history folds it, so a page
boundary changes what was read, never the answer.

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

The pinned head is the whole call: a compaction during the call cannot move
it and the pages all read at that head. A facade read while the Session reloads
returns `HostError::Cancelled("capability reloading")`, any other facade error
is a failed call, and the call's cancellation token is checked between pages and
before rendering, as `wait` does. Pages are 20 entries, or 8,000 characters of
one thing in full; every variable part of a line is cut at 120 characters, so
a line stays at most 600 characters. The tool returns text the model reads and
`structuredContent` for UI and persistence only.

### Tests

[recall_flow.rs](../../../../crates/dim-agent-capabilities/tests/recall_flow.rs)
holds the acceptance flows of the design's §14: the turn index and its pages,
a turn, one thing in full with CJK paging, the allowlist against what context
assembly places, search, compactions, call endings, branch and Session forks,
line bounds, the read-only preset, a long turn, interrupted outputs, the
argument errors, a 120-turn history whose index, search and newest thing
answer from bounded pages, and a child branch paging through its inherited
prefix.
[recall_pages.rs](../../../../crates/dim-agent-capabilities/tests/recall_pages.rs)
counts the folds of a no-match search over two history lengths: the count stays
at two, so the work does not grow with the pages read.
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
