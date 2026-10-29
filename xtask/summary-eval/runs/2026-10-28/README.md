# #170 summary evaluation: per-run records

The runs behind the [research note](../../../../docs/design/research/summary-eval.md)
([中文](../../../../docs/zh/design/research/summary-eval.md)). One JSON object per line, one line per
run, one file per class and phase of the evaluation.

## How these files were made

The evaluation runner (`cargo run --locked -p xtask -- summary-eval …`) wrote one JSON file per run
under a directory outside the repository; the runner's output is unchanged since. The files were
assembled by a script from those records, selecting fields and applying the two corrections below:
no text was added, rewritten or summarized on the way in, and the summaries and the judge's quotes
are the models' output as the runner recorded it.

Nothing was removed for publication: the recorded runs held no credential, no home or local path,
no URL and no gateway or service name, and a scan for each of them found none.

## Files

| File | Runs | Bytes |
| --- | --- | --- |
| `openai-compatible-phase1.jsonl` | 72 | 240,892 |
| `openai-compatible-phase2.jsonl` | 120 | 424,259 |
| `non-reasoning-control-phase1.jsonl` | 72 | 245,445 |
| `non-reasoning-control-phase2.jsonl` | 120 | 246,487 |
| `openai-responses-phase1.jsonl` | 19 | 22,826 |
| **Total** | **403** | **1,179,909** |

## A line

| Field | Meaning |
| --- | --- |
| `class` | `openai-compatible`, `non-reasoning-control`, or `openai-responses` |
| `phase` | `1` for the reasoning arms, `2` for the target-length arms |
| `arm` | `reasoning-inherit`, `reasoning-off`, `reasoning-low`, `target-none`, `target-25-5`, `target-25-10`, `target-50-5`, `target-50-10` |
| `case`, `run` | the case id from [`../../cases/`](../../cases/) and the run index, `0` to `2` |
| `outcome`, `error` | `completed`, `failed` or `skipped`, and the error text when a compaction failed |
| `summary` | the summary the run stored, as stored (the escaped envelope) |
| `probes` | one entry per probe of the case: `id`, `kind` (`fact`, `exact`, `absent`), `text`, `judge` (the verdict the run recorded), `corrected` (see below), `quote` (the judge's supporting quote, when it gave one) |
| `scores` | per kind, `[present, total]` over this run's probes: `F` over `fact`, `X` over `exact`, `V` over `absent` |

## The corrected verdicts

Two corrections separate `judge` from `corrected`:

- `fact` and `absent`: the hand corrections the operator recorded before deciding — one `absent`
  probe of the `instructions` case that the judge read as `present` and the operator corrected to
  `absent`, 14 verdicts in all.
- `exact`: recomputed from the stored summary with the runner's fixed matching — the probe goes
  through the producer's escape table (`&`, `<`, `>`), both sides are whitespace-normalized, and
  the summary is compared as stored. The recorded `judge` value is what the pre-fix runner wrote.

A failed run has no summary, so its `fact` and `exact` probes count as missed and its `absent`
probes as not violated.

## Reproducing the note's numbers

Every pooled figure in the note is the sum of one `scores` kind over an arm's lines, except the
medians of `T` and `durationMs`, which the runner reports per run and these files do not carry. For
example, the OpenAI-compatible phase 1 arms come out as `inherit` `F` 69/72 and `X` 23/24, `off`
70/72 and 24/24, `low` 69/72 and 24/24.
