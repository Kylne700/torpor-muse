# main-1: the scored run of the pre-registered study

The one scored run of the study fixed in [PREREG.md](../../PREREG.md) (written 2026-10-26, before any run). It is published whatever it says: all 168 rows, the analysis output as written, the plan and the spend lines. Excluded units: none (168 of 168 rows valid, 0 excluded, 0 pending, 42 of 42 groups complete, not stopped early; `summary.json`).

The standard-model subset of the same study (arms A and D only, 9 pairs, model `muse-spark-1.3`, directional only) is a separate folder, [subset-1](../subset-1/README.md); it is not pooled with this run.

## Result

On Muse Code 1.4.13 (1.4.13-R6996.1), with model id `muse-spark-1.3-contributor`, on 42 valid pairs (21 synthetic tasks, 2 repetitions each), the adapter arm kept the app's behaviours (`final_pass`) in 34 of 42 episodes against 32 of 42 for the engine hand-wired without the adapter. The verdict word, as written by the analysis script: **inconclusive at this n**.

| comparison | first arm | second arm | b / c | p (exact McNemar) | effect, 95 % CI | verdict word |
|---|---|---|---|---|---|---|
| D vs C (primary): adapter plus engine vs engine only | 34/42 | 32/42 | 3 / 1 | 0.625 | +0.05, [-0.05, +0.14] | inconclusive at this n |
| D vs A (secondary): adapter plus engine vs nothing | 34/42 | 34/42 | 2 / 2 | 1.000 | 0.00, [-0.10, +0.14] | none (secondary) |
| B vs A (secondary): notes file vs nothing | 36/42 | 34/42 | 3 / 1 | 0.625 | +0.05, [-0.05, +0.19] | none (secondary) |

Every other table (by kind of task, other outcomes) is in [analysis.md](analysis.md), also as written. The limits were stated in advance, [PREREG.md section 10](../../PREREG.md): tasks, oracles and injections written by one AI agent; synthetic data (HTTP/JSON Node apps, 6-turn episodes); one model family and one Muse Code build; repetitions of a task are not independent, so the interval resamples the 21 tasks and is wide.

## Build

- Adapter commit `0e8b0da` (2026-10-27), engine torpor 2.3.1, Muse Code 1.4.13 (1.4.13-R6996.1), model `muse-spark-1.3-contributor`, mode keyed. Run 2026-10-27 04:33 to 08:21 UTC, run seed 20261027.
- This is an earlier build than the one measured in [e2e/PROOF.md](../../../e2e/PROOF.md), 8b119d6. `git log 0e8b0da..8b119d6 -- src/` lists what changed in between (72 commits).
- Analysis: [eval/analysis/analyze.mjs](../../analysis/analyze.mjs), sha256 `63939820e82c3ce52a4e361adee8d02d4724cdb8a011e0f61dd5589724a5e0ea` (the same file as in this repository), analysis seed 1, 10,000 resamples.
- Spend, upper bound from token counts: USD 31.512686 (Muse Code calls 26.976113, check-writer calls 4.536573; `summary.json`, `spend.jsonl`).

## Files

| file | what it is |
|---|---|
| `analysis.md`, `analysis.json` | the analysis output as written |
| `analysis-console.txt` | the console text of the same run (last line edited, see Masks) |
| `plan.json` | the run plan: tasks, arms, repetitions, order |
| `summary.json` | totals of the run |
| `spend.jsonl` | one line per turn: `t`, `ep`, `turn`, `usd_hi`, `usd_lo`, `writer_usd_hi`; whole, no field dropped (1,005 lines) |
| `rows/` | 168 files, one per episode attempt, named `task__arm__rep__attempt.json` |

## Masks

Two path edits, nothing else. The other 131 of the 174 data files are byte-equal to the originals.

1. `analysis-console.txt`, last line: the folder of the run on the author's machine is written `<results folder>`.
2. The 42 arm C rows (`rows/*__C__*.json`), in the ten hook commands of `manifest.settings`: the path of the key file inside the episode sandbox, which began with the sandbox folder (`<episode>`) and its `home` folder in the run, is written `<episode home>/.config/torpor/key`, as [PREREG.md](../../PREREG.md) writes it. `settings_sha256` in these rows is the hash of the settings file as it was written during the run, before this edit; the settings files are not published.

Check that the masks changed no number: `node eval/analysis/analyze.mjs --rows eval/results/main-1/rows --md /tmp/a.md --json /tmp/a.json --seed 1 --boot 10000`, then `cmp /tmp/a.md eval/results/main-1/analysis.md` and the same for the JSON. Both were byte-equal when this folder was made.

## Not published

- `episodes/`: the 168 session logs (`proxy.log` and `session-1.jsonl.gz` per episode, 65.9 MB). [PREREG.md section 11](../../PREREG.md) says the logs are published only if Meta's terms allow it, and otherwise stay unpublished with this stated. Meta's public terms do not say whether the system and developer messages of Muse Code inside such logs may be published, and the logs also hold paths on the author's machine. So they stay unpublished.
- The runner's operational report (it names files outside this repository).
