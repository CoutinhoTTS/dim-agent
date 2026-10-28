# Summary evaluation: reasoning and target length

Status: research result for [#170](https://github.com/arcships/dim/issues/170). It evaluates the
two choices of [ContextCompaction](../plugins/ContextCompaction.md) §13; the decisions are
recorded there (§10, §13.2, §13.3). Chinese copy:
[中文](../../zh/design/research/summary-eval.md).

Every statement carries its kind of evidence:

- **[Measured]**: measured by the evaluation runs; the method is in §2.
- **[Code]**: read from this repository.
- **[Judgment]**: the reasoning or inference of this note; not verified.

## 1. Result

| # | Question | Decision | Evidence |
| --- | --- | --- | --- |
| 1 | Reasoning of summarizer requests | `off` | §3: `off` passes on the measured reasoning class with no quality loss (`F` 0.972 vs 0.958, `X` 1.000 vs 0.958, `V` 0, no failures) |
| 2 | Introduce the setting? | No: `off` is a strategy constant and `compaction.json` is not introduced | §3: `off` is also the best arm on the class that ran |
| 3 | Target length instruction | None | §4: every phase-2 arm passes, and the no-instruction arm's cut-offs and failures are no higher than any target arm's |

The rule and `δ = 0.05` are §13.4's. The decisions rest on the reasoning class that ran; the
OpenAI Responses class could not run (§2.3).

## 2. Method

### 2.1 Classes, model and judge

| Class | Endpoint | Result |
| --- | --- | --- |
| effort-based reasoning | OpenAI-compatible | ran, phases 1 and 2 |
| effort-based reasoning | OpenAI Responses | **could not run** (§2.3) |
| control: requested effort produces no thinking | Anthropic-compatible | phase 1 ran; phase 2 not used (§4.2) |

[Measured] The model is DeepSeek v4.1 flash, reasoning max, through its OpenAI-compatible
endpoint. Its catalog entry declares a window of 1,048,576 tokens and a maximum output of 8,192.
Its saved preference for the conversation is reasoning effort `max`, and the `inherit` arm
inherited it (§2.4).

[Measured] The judge, including for the control class, is the same model family on the same
endpoint, at temperature 0. It sees the summary and the probe list only, never the transcript.
The judge and the summarizer being one family is a limitation of this evaluation (§9).

[Measured] The control is the same model through its Anthropic-compatible endpoint, where the
requested effort produced no thinking: the three arms sent three request shapes (`inherit`
declared nothing, `off` the native `none` effort the mapping turns into a disabled-thinking
declaration, `low` an effort of `low`), and all 161 recorded replies of phase 1 contained no
thinking at each of the three values.

### 2.2 Cases and metrics

[Measured] The eight committed cases of the runner (§13.4), including the five-step `chain`, and
three runs per case per arm. `F` is fact recall over the `fact` probes, `X` exact recall over the
`exact` probes, `V` violations of `absent` probes, `C` replies cut off at `max_tokens`, `T` the
summary's estimated tokens and `S` the target length of a target arm. The decision rule,
including `δ = 0.05`, is §13.4's.

### 2.3 The OpenAI Responses class could not run

[Code] The request builder asks for the native reasoning continuation data
(`include: ["reasoning.encrypted_content"]`, `crates/dim-agent-aimux/src/request/native.rs:233-234`)
and, for an effort other than `none`, a reasoning summary. [Measured] Of the 20 recorded replies,
19 carried a reasoning summary and none carried the encrypted content. [Code] The reply reader
fails any finished reply whose thinking parts lack that continuation data
(`crates/dim-agent-aimux/src/stream.rs:261-273`), so 18 of the 19 recorded runs of the `inherit`
arm failed with "Model thinking is missing native continuation metadata"; the one that completed
is the one whose reply carried no reasoning. The class therefore measures the endpoint-driver
pairing, not summary quality, and no arm of it enters a decision. The rule is rerun for that
class when it can run.

### 2.4 Recording, and the runner defects

[Measured] A pass-through recorder in front of the endpoint captured every request the runner
sent and every reply, removing credentials before writing. This shows, from the runner's own
requests, that `inherit` inherited `max` before each class ran: the first summarizer request of
the OpenAI-compatible class carried `reasoning_effort: "max"`, the one of the Responses class
carried `reasoning: {"effort": "max", "summary": "detailed"}`, and the control's carried neither
`thinking` nor `output_config`; each run's diagnostics recorded `reasoning: "inherit"`.

Two runner defects were fixed in [#545](https://github.com/arcships/dim/pull/545): the
conversation and judge configs were built with default preferences instead of the model's saved
ones, and `exact` probes were compared with the escaped summary, which undercounted `X`; every
`X` here is under the fixed rule, which unescapes the summary text one level before the
whitespace-normalized comparison.

## 3. Phase 1: reasoning

### 3.1 OpenAI-compatible class

[Measured] After the hand corrections of §6 (`F` denominator 72, `X` denominator 24):

| Arm | fell back | F | X | V | C | failed | median T | median duration ms |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `inherit` | 0 | 69/72 = 0.958 | 23/24 = 0.958 | 0 (raw 1) | 0 | 0 | 394 | 3,235 |
| `off` | 0 | 70/72 = 0.972 | 24/24 = 1.000 | 0 (raw 1) | 0 | 0 | 428 | 1,460 |
| `low` | 0 | 69/72 = 0.958 | 24/24 = 1.000 | 0 (raw 1) | 0 | 0 | 502 | 2,237 |

The raw `V` of 1 in each arm is the same judge error on the `instructions` case (the summary
marked a withdrawn rule as initial, the judge counted it as current), corrected in §6. The
remaining `exact` miss in `inherit` is a real omission: the summary of one `cjk` run does not
contain the `targetTokens` identifier.

### 3.2 Control class

[Measured] The same rule, on the non-thinking endpoint:

| Arm | fell back | F | X | V | C | failed | median T | median duration ms |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `inherit` | 0 | 69/72 = 0.958 | 24/24 = 1.000 | 0 (raw 1) | 0 | 0 | 442 | 1,433 |
| `off` | 0 | 72/72 = 1.000 | 23/24 = 0.958 | 0 (raw 1) | 0 | 0 | 446 | 1,446 |
| `low` | 0 | 71/72 = 0.986 | 24/24 = 1.000 | 0 (raw 2) | 0 | 0 | 427 | 1,361 |

The arms spread by 0.042 in `F` and 0.042 in `X`, both below `δ`, with no cut-offs, failures or
unjudged probes: run-to-run noise stays under the threshold it is compared against. The `off`
arm's remaining `exact` miss is a real omission, as in §3.1.

### 3.3 Decision

A reasoning arm passes on a class when `F ≥ F_inherit − δ`, `X ≥ X_inherit − δ`,
`V ≤ V_inherit` and its failures are no more than `inherit`'s; the default is the first of
`off`, `low`, `inherit` that passes on every class.

[Measured] On the OpenAI-compatible class (`inherit`: `F` 0.958, `X` 0.958, `V` 0, failures 0):

- `off`: `F` 0.972 (`+0.014`, threshold 0.908) ✓; `X` 1.000 (`+0.042`) ✓; `V` 0 ≤ 0 ✓;
  failures 0 ≤ 0 ✓ → passes.
- `low`: `F` 0.958 (`+0.000`) ✓; `X` 1.000 (`+0.042`) ✓; `V` 0 ≤ 0 ✓; failures 0 ≤ 0 ✓ →
  passes.
- `inherit` passes as itself.

The first passing value is **`off`**. The control agrees: `off` there has `F` 1.000 (`+0.042`),
`X` 0.958 (`−0.042 ≥ −0.05`), `V` 0 and no failures, so it passes there too.

### 3.4 Is the setting introduced?

[Measured] On the class that ran, `off` is also the best arm: it has the highest `F` (0.972;
`low` and `inherit` 0.958) and the highest `X` (1.000). No other arm's `F` is more than `δ`
above it. The setting is therefore **not introduced**: `off` becomes a strategy constant, and
`compaction.json` does not exist. The second reasoning class could not run (§2.3), so this
statement covers the class that did.

## 4. Phase 2: target length

Run with `--reasoning off`. `S = min(α × O, β × B)` rounded down to a multiple of 100, where
`O` is the output allowance and `B = window − O`. For the seven cases at the model's own window
(`O` 8,192, `B` 1,040,384) the `β` shares never bind at the tested values, so
`S(0.25, 0.05) = S(0.25, 0.10) = 2,000` and `S(0.5, 0.05) = S(0.5, 0.10) = 4,000`. The
`folding` case narrows the window to 8,000, so `O` is 2,000, `B` 6,000, and the four rules give
300, 500, 300 and 600.

### 4.1 OpenAI-compatible class

[Measured] After the hand corrections of §6:

| Arm | F | X | V | C | failed | median T | median T/S | p90 T/S |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| no instruction | 71/72 = 0.986 | 24/24 = 1.000 | 0 | 0 | 0 | 459 | – | – |
| 0.25/0.05 | 71/72 = 0.986 | 24/24 = 1.000 | 0 (raw 2) | 0 | 0 | 448 | 0.242 | 1.403 |
| 0.25/0.10 | 70/72 = 0.972 | 24/24 = 1.000 | 0 (raw 2) | 0 | 0 | 464 | 0.232 | 1.074 |
| 0.50/0.05 | 71/72 = 0.986 | 24/24 = 1.000 | 0 (raw 1) | 0 | 0 | 453 | 0.125 | 1.257 |
| 0.50/0.10 | 69/72 = 0.958 | 24/24 = 1.000 | 0 (raw 2) | 0 | 0 | 487 | 0.132 | 0.777 |

Every arm passes against the best arm: `F ≥ 0.986 − 0.05 = 0.936` (the lowest is 0.958),
`X ≥ 1.000 − 0.05 = 0.950` (every arm is 1.000), `V ≤ 0`. The no-instruction arm has `C` 0 and
0 failures, no higher than any passing target arm (all 0), so **no instruction is kept**; no
target is selected and the p90 rule does not bind.

### 4.2 Control phase 2

The endpoint began returning reasoning despite the off declaration partway through, so that
phase is not used.

## 5. Planning-size evidence

[Measured] For the no-instruction arm of §4.1, each run's `T` against the planned
`replacement_size` at that run's window: `S` at α = 0.5, β = 0.10 (§13.3) plus the empty
summary wrapper, which is 84 tokens. At the model's own window the plan is 4,084 tokens, and on
the narrowed `folding` window 684. Over the 24 runs:

| Measure | Value |
| --- | --- |
| median `T / planned` | 0.11 |
| p90 `T / planned` | 0.67 |
| runs above the plan | 0 of 24 |
| maximum | 0.89 (`folding`, one run) |

Outside the narrowed case every ratio is at most 0.20; the p90 and the maximum come from the
`folding` case, whose 600-token plan is small. A run above the plan is not fatal: §5.3 step 4
re-plans once at the replacement's actual size before anything is committed.

## 6. Hand corrections

Before the decision, the operator read the lowest-scoring summaries and corrected judge errors
by hand. Every correction was the same `absent` probe in `instructions`, whose statement must
not appear as a current rule; the summaries quoted it as an initial, superseded instruction, and
the judge counted the appearance as a violation. In total 14 verdicts were corrected from
`present` to `absent`: 3 in the OpenAI-compatible phase 1, 4 in the control phase 1 and 7 in the
OpenAI-compatible phase 2. Representative quotes:

| Run | Quote from the summary |
| --- | --- |
| OpenAI-compatible phase 1, `inherit` | "The user initially said the runner could `unwrap` freely because it would never be called from production." |
| OpenAI-compatible phase 2, 0.50/0.05 | "**Initial instruction:** The user said the runner could use `unwrap` freely because it is a tool never called from production." |
| Control phase 1, `low` | "Initial instruction: the runner is tool code that will never be called from production, so it may use `unwrap` freely…" followed by "Correction from team review: no `unwrap` outside tests…" |

No other judge error survived the review. One observation is recorded but not corrected: the
`cjk` `constraint` probe asks for "no comments" while the transcript and the summary say "no
English comments", so the probe is stricter than the source, and the judge's `absent` is
literally correct.

## 7. Per-item recall and the chain

[Measured] `constraint` is the only item class that is consistently below 1.000 (0.833–1.000
across arms and classes); `task` misses once in a few arms; `decision`, `openItem`, `judgment`
and `nextStep` are perfect everywhere. In the `chain` case, whose probes are judged on the
fifth summary, all 33 recorded chains completed their five steps: the standing constraint
survived in 31 of 33 (one OpenAI-compatible phase-2 run dropped it, one control phase-1 run wrote
it under "superseded/withdrawn"), the withdrawn rule was never presented as current (33 of 33),
and the replacement constraint and the refined task were present in all 33.

## 8. Cost

[Measured] Tokens are the prompt and completion counts the replies reported; three replies
reported none and count as zero tokens but as requests.

| Class / phase | summary requests | judge requests | prompt tokens | completion tokens | total tokens |
| --- | --- | --- | --- | --- | --- |
| OpenAI Responses, phase 1 (stopped arm, partial) | 20 | 1 | 34,697 | 12,270 | 46,967 |
| OpenAI-compatible, phase 1 | 157 | 72 | 486,598 | 164,956 | 651,554 |
| OpenAI-compatible, phase 2 | 262 | 121 | 822,725 | 251,450 | 1,074,175 |
| Control, phase 1 | 161 | 72 | 340,575 | 158,510 | 499,085 |
| Control, phase 2 (mostly the failed replies) | 178 | 47 | 434,142 | 134,922 | 569,064 |
| **Total** | **778** | **313** | **2,118,737** | **722,108** | **2,840,845** |

## 9. Limitations

- The judge is the same model family as the summarizer. §13.4 asks for a different family where
  possible; this evaluation used one family, so judge bias toward its own family is not
  excluded.
- One reasoning class was measured; the OpenAI Responses class is blocked by the
  endpoint-driver pairing (§2.3), so the cross-class statement covers the class that ran.
- The control's non-thinking behaviour is the endpoint's, at the three requested values during
  phase 1. The same endpoint returned reasoning from a point in phase 2 onward, so that phase is
  not used (§4.2).
- `X` here is under the fixed runner rule; the earlier escaped-form readings undercounted it.
