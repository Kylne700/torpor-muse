# subset-1: the standard-model subset of the pre-registered study

The subset that [PREREG.md section 9](../../PREREG.md) (line 118) puts into the study: arms A and D only, 9 episodes per arm (1 repetition), on 9 tasks, run on the standard model id `muse-spark-1.3`, at most USD 45. It is published whatever it says: [PREREG.md section 11](../../PREREG.md) (line 142) promises all rows and the analysis output as written, and this folder holds all 18 rows, the analysis output as written, the plan and the spend lines. Excluded units: none (18 of 18 rows valid, 0 excluded, 0 pending, 9 of 9 groups complete, not stopped early; `summary.json`). The one scored run of the study is [main-1](../main-1/README.md).

## Result

Labelled "standard model, directional", as [PREREG.md section 9](../../PREREG.md) asks. On Muse Code 1.4.13 (1.4.13-R6996.1), with model id `muse-spark-1.3`, on 9 valid pairs (9 synthetic tasks, 1 repetition each), the pass counts of `final_pass` (the app's behaviours kept) are 8 of 9 episodes for the adapter arm and 6 of 9 episodes for the arm with nothing added. The analysis script prints no verdict word for this comparison, because D vs A is secondary: the verdict column reads "none (secondary)". A plain reading, not a script output: inconclusive at this n.

| comparison | first arm | second arm | b / c | p (exact McNemar) | effect, 95 % CI | verdict word |
|---|---|---|---|---|---|---|
| D vs A (secondary): adapter plus engine vs nothing | 8/9 | 6/9 | 2 / 0 | 0.500 | +0.22, [0.00, +0.56] | none (secondary) |

b = pairs where the adapter arm passed and the other failed, c = the reverse. D vs C (primary) is not computed: arm C, the engine without the adapter, was not run in the subset, and the script says so (`D vs C (primary) is not computed: the folder has no rows for arm C`, in the Warnings of [analysis.md](analysis.md)). So nothing in this folder separates the adapter from the engine.

Directional only, with no significance claim, and not pooled with the main run ([PREREG.md section 9](../../PREREG.md)). For comparison: main-1 ran on the model id `muse-spark-1.3-contributor` (21 tasks, 2 repetitions each, 42 pairs), and its D vs A was 34/42 against 34/42, effect 0.00, 95 % CI [-0.10, +0.14] ([main-1/analysis.md](../main-1/analysis.md), section 1). Every other table (by kind of task, other outcomes, per task) is in [analysis.md](analysis.md), also as written. The limits were stated in advance, [PREREG.md section 10](../../PREREG.md): tasks, oracles and injections written by one AI agent; synthetic data (HTTP/JSON Node apps, 6-turn episodes); one model family and one Muse Code build. Here each of the 9 tasks ran once per arm, so the interval resamples 9 tasks and is wide.

## Build

- Adapter commit `0e8b0da` (2026-10-27), engine torpor 2.3.1, Muse Code 1.4.13 (1.4.13-R6996.1): the same build as main-1. Model `muse-spark-1.3`, mode keyed. Run 2026-10-27 08:26 to 08:51 UTC, run seed 20261027.
- This is an earlier build than the one measured in [e2e/PROOF.md](../../../e2e/PROOF.md), 8b119d6. `git log 0e8b0da..8b119d6 -- src/` lists what changed in between (72 commits).
- Tasks, the 9 that [PREREG.md section 9](../../PREREG.md) names (3 per app): notes-ctrl, notes-dup, notes-logout, shop-clear, shop-coupon, shop-merge, todo-ctrl, todo-persist, todo-tags.
- Analysis: [eval/analysis/analyze.mjs](../../analysis/analyze.mjs), sha256 `63939820e82c3ce52a4e361adee8d02d4724cdb8a011e0f61dd5589724a5e0ea` (the same file as in this repository and as for main-1), analysis seed 1, 10,000 resamples.
- Spend, upper bound from token counts: USD 39.152913 (Muse Code calls 38.201161, check-writer calls 0.951752; the sums of the rows, equal to `usd_hi` in `summary.json`). The 108 lines of `spend.jsonl` add up to 39.152912 (`spend_total_usd_hi` in `summary.json`); the 0.000001 is rounding, because each row rounds its episode total to 6 decimals.

## Files

| file | what it is |
|---|---|
| `analysis.md`, `analysis.json` | the analysis output as written |
| `analysis-console.txt` | the console text of the same run (last line edited, see Masks) |
| `plan.json` | the run plan: tasks, arms, repetitions, order |
| `summary.json` | totals of the run |
| `spend.jsonl` | one line per turn: `t`, `ep`, `turn`, `usd_hi`, `usd_lo`, `writer_usd_hi`; whole, no field dropped (108 lines) |
| `rows/` | 18 files, one per episode attempt, named `task__arm__rep__attempt.json` |

## Masks

One path edit, nothing else. The other 23 of the 24 data files are byte-equal to the originals.

1. `analysis-console.txt`, last line: the folder of the run on the author's machine, which the line names twice, is written `<results folder>`.

main-1 has a second edit, for the key-file path in its arm C rows. The subset has no arm C, and no row here holds a key-file path or a path on the author's machine.

Check that the published analysis is what the published rows give: `node eval/analysis/analyze.mjs --rows eval/results/subset-1/rows --md /tmp/a.md --json /tmp/a.json --seed 1 --boot 10000`, then `cmp /tmp/a.md eval/results/subset-1/analysis.md` and the same for the JSON. Both were byte-equal when this folder was made.

## Not published

- `episodes/`: the 18 session logs (`proxy.log` and `session-1.jsonl.gz` per episode, 6,978,360 bytes). [PREREG.md section 11](../../PREREG.md) says the logs are published only if Meta's terms allow it, and otherwise stay unpublished with this stated. Meta's public terms do not say whether the system and developer messages of Muse Code inside such logs may be published (all 18 logs hold such messages), and the logs also hold paths on the author's machine (18 of 18) and the machine's user name (9 of 18). So they stay unpublished.
- The runner's operational report (it names files outside this repository).
