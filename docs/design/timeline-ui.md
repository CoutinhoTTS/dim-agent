# Timeline-first TUI: branches and forks derived from the Timeline

Status: Proposed. Builds on [Timeline branches](timeline-branches.md) ([#427](https://github.com/arcships/dim/issues/427)) and [Session fork](appserver/session-deletion-and-fork.md). Scope: the CLI TUI's information hierarchy and interaction, and the client-side data it folds. No runtime, event, storage or App Server method changes. Chinese: [中文](../zh/design/timeline-ui.md).

## 1. Problem

After #427 the TUI had three unrelated entries: the conversation (`main`'s transcript), a `/lines` panel next to `/tasks`, and no TUI entry for `session.fork` at all. A user could not see where a line grew from, and could not open a line from a point in history (#427 S3).

In the data these are one thing: **a branch and a fork both cut a Timeline at a sequence.** `branch/create` carries `at`, and both `session.branch_create {parentBranchId, at, prompt}` and `session.fork {sessionId, at}` take `at`. The Timeline is therefore the primary axis; branching and forking are actions on a point of it; the lines panel is the map of what those actions produced.

## 2. Hierarchy

```
L0  Session ............ the subject: identity, workspace, interaction surface
 └ L1  Timeline ........ the shown line's transcript; the only main surface
     ├ point ........... a cut: the turn boundary just before a user message
     │   ├ e edit / q quote        (existing)
     │   ├ b branch here → a line in this Session
     │   └ f fork here   → a new Session (main only)
     ├ marker .......... ⑂ a line opened here (pickable)
     └ /timeline ....... every point of the shown line, to jump to (§4.6)
 L3  Lines (/lines) .... every line under the line it was cut from
```

1. **One main surface.** The screen always shows one line's Timeline. Changing lines changes which Timeline is shown, not the screen.
2. **Derive from a point.** Branching and forking start from a point of the shown Timeline. Its end is a point too (`/branch`, `/fork`).
3. **Results return to a Timeline.** A branch leaves a `⑂` marker on its parent and the screen follows it; a fork attaches to the new Session's Timeline.

## 3. Branch versus fork

Both share the point → derive entry. They differ in where the result lives (#427 §2):

| | Branch (`b`) | Fork (`f`) |
| --- | --- | --- |
| Result | A line of this Session | A new Session |
| Shares | The workspace and the subject; talks with its parent | History up to the cut only |
| Points | Any open line, at a valid context boundary | `main` only, at a rest point |
| After | The screen shows the new line | The TUI attaches to the fork |
| Trace on the source | A `⑂` marker | None (§7) |

An action that does not apply says why in a notice (`A fork copies main; b opens a line here`, `Line has ended: X`). The App Server stays the only judge of a valid cut; its refusal is shown as a notice.

## 4. Timeline and points

### 4.1 Point

The fullscreen pick (Tab) already stops on user messages. A picked message's point is the sequence just before it: "before this message was said". It ends an atomic batch, usually passes `validate_context_boundary`, and on `main` is usually a rest point; branching there with a reworded message is S3's retry-and-compare. A message queued while a turn ran cuts before its submission, not where it entered the transcript; that cut lies inside the running turn, so a fork there is refused, and a line only where the context boundary allows.

The end of the shown line is a point as well: its newest turn end, used by `/branch` and `/fork`. With no finished turn there is no point (`No finished turn to cut at yet`).

```
┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ › try RwLock for the cache lock                           ┃
┗━e edit · b branch · f fork━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
```

A box too narrow for the hint shows `e edit`.

### 4.2 `b`: branch here

`session.branch_create` needs the line's opening input, so `b` puts the picked message in the composer and the composer's next submit opens the line:

```
╭──────────────────────────────────────────── provider / model ─╮
│ › try RwLock for the cache lock█                              │
╰───────────────────────────── ⑂ new line from main · Esc cancel ─╯
```

`Enter` sends `session.branch_create {parentBranchId: shown line, at, prompt}`, and the screen shows the new line. `Esc` cancels. A session or line switch drops the draft.

### 4.3 `f`: fork here

On `main` only. The composer's border asks `Fork the session before this message? (y/N)`; `y` sends `session.fork {sessionId, at}` and attaches to the fork under a `forked ID · N messages` boundary, with the picked message in its composer. A fork starts no work until the user submits (fork design §6.5).

### 4.4 Markers

`branch/create` is written on the parent at the moment the line opens, which for a past cut is the parent's tail, not the cut. Committed scrollback is append-only, so the marker stays where the fact is and names its cut instead:

```
 › find the leak
 … reply …
 › try the parser
 … reply …
 ⑂ line f7f0a984 · before “try the parser” · try the parser via cache
```

`before “…”` appears when the cut is just before a message of this transcript; a line opened at the tail (a `branch` tool call) has none. The marker is a pick stop like a member card: `Enter`, or a click, shows the line. Ends are not marked: the lines panel shows them, and `main` already receives notices for its tool lines.

### 4.5 Where am I

While a line is shown, the composer's bottom border names its path from `main` and its cut:

```
╰── ~/repo (main) · line main ▸ b-91 ▸ b-c3 · after message 4 ─╯
```

`after message N` counts the user messages in the parent's history, its inherited ones included, at the cut.

### 4.6 `/timeline`: jumping to a message

The Timeline's points are its user messages. Tab walks them one at a time from the newest; `/timeline` lists them all, as the desktop client's message rail does: each row is the message and the first text of its answer.

```
┌ Timeline ────────────────────────────────────────────────────────┐
│   #1  weather in Shanghai      Cloudy, 20°C, light wind …        │
│   #2  try RwLock for the cache The lock moves into …             │
│ › #3  adsa                                                       │
└──── Enter jump · e edit · q quote · b branch · f fork · Esc close ┘
```

- The newest message is selected. Typing does not filter: the letters act on the selected message as they do on a pick (§4.1–4.3).
- `Enter` jumps. Fullscreen picks the message, as Tab would, and the pick keys apply. Scrollback cannot move up: a message it has not drawn yet is drawn from there on as an earlier-history page; one already drawn gets a notice that `/tui` can jump to it.

A resume draws the newest 100 messages. Fullscreen tops the transcript with a load-more row (`↑ 130 earlier messages · Enter or click to load`): a click, or Tab up to it and `Enter`, draws the previous 100 above and keeps the oldest drawn message at the top of the view. Scrollback has no such row; its earlier history comes from `/timeline` jumps. This replaces `/history`, which paged earlier messages in after the newest ones.

## 5. Switching lines

Unchanged from #427 §14: a switch writes a `line ID` boundary and the shown line's transcript, which holds its own events only (each branch numbers its own sequences, [sessions service D11](appserver/sessions-service.md)). Its inherited history is its parent's, one marker away.

## 6. Lines panel

`/lines` is the map of the Timelines. Children are ordered by where they were cut from their parent, then by opening order, and each row says `after message N`:

```
┌ Lines ───────────────────────────────────────────────────────────┐
│   main        the session itself                                 │
│     b-02      after message 1 · docs pass · ended itself         │
│     b-7c      after message 3 · parser race · working · shown    │
│       b-c3    after message 4 · the guard in cache.rs · working  │
└──────────────────── Enter show the line · x end it · Esc close ──┘
```

Forks are other Sessions and belong to `/resume`. Keys stay as #427 §14 defines them.

## 7. Client data

| # | Need | Owner | Notes |
| --- | --- | --- | --- |
| R1 | `LineView.cut`: the parent sequence and the message count at it | CLI projection | Folded from `branch/create.at` and the parent's user inputs; no format change |
| R2 | A user message row's cut (`ViewRow.cut`) and a marker row's line (`ViewRow.line`) | CLI projection | From sequences the client already has |
| R3 | `session.branch_create` and `session.fork` calls | CLI session host | Existing methods; a request made from a stale attachment is refused locally |
| R4 | Later, optional: show a fork's origin (`↗ forked` on the source, `forked from S` in the fork) | App Server, needs a format decision | `session/forked` carries no data and the fork design adds no provenance column |

## 8. Not in scope

- **Rewinding `main`.** #427 §3: not a branch. "Start over from here" is `b` (compare in this Session) or `f` (a new Session).
- **Compare and adopt.** S3's second half needs its own design. Sibling lines cut at one point are what a comparison will show.
- **A drawn DAG.** Scrollback is append-only; topology is markers, the panel and the path.

## 9. Ablation review

- No new mode: points reuse the Tab pick, line markers and the load-more row reuse the card pick, the switch reuses #427's, the fork's attach reuses `/resume`'s. The one new panel, `/timeline`, lists the points the pick walks and takes the pick's keys.
- New interaction: `b`/`f` on a pick, the composer's line draft and fork question, the path border, `/branch` and `/fork`. Each serves one rule of §2.
- Considered and dropped: a `b` key in the lines panel (needs each line's head; `/branch` on the shown line covers it), greyed hints per point (a notice on the key says the same with less state), end markers (§4.4), a one-row summary of the inherited prefix and `Esc Esc` back to the parent (§5 and the marker already lead there).

## 10. Decisions

1. A point is the boundary before a user message, plus the line's end. A boundary after an answer is not offered.
2. `b` and `f` carry the picked message into the composer, as `e` does.
3. R4 waits for a format decision.
