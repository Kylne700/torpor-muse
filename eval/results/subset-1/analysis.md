# Eval results

*Source: model `muse-spark-1.3`; run id `subset-1`.*

Rows read: 18. Units (task x arm x rep): 18; valid 18, excluded, no valid attempt (3 failed attempts, or one not re-run) 0, pending 0.
Analysis: `eval/analysis/analyze.mjs`, sha256 `63939820e82c3ce52a4e361adee8d02d4724cdb8a011e0f61dd5589724a5e0ea`; bootstrap seed 1, 10000 resamples; Wilson z = 1.959964.

Warnings:
- D vs C (primary) is not computed: the folder has no rows for arm C

## 1. Pre-registered comparisons (final_pass, all kinds pooled)

*Source: model `muse-spark-1.3`; run id `subset-1`.*

| comparison | pairs | tasks | first arm passes | second arm passes | b | c | p (exact McNemar) | effect | 95 % CI (cluster bootstrap) | verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| D vs A (engine + new work) | 9 | 9 | 8/9 | 6/9 | 2 | 0 | 0.500 | +0.22 | [0.00, +0.56] | none (secondary) |

b = pairs where the first-named arm passed and the second failed; c = the reverse; effect = (b - c) / pairs, the difference in pass rate.
p = exact two-sided McNemar on b + c. CI = 95 % percentile interval of a cluster bootstrap that resamples TASKS with replacement.
Verdict words are fixed in advance and only the primary comparison (D vs C) gets one: "D better than C" only if p < 0.05 AND the whole interval is above 0; otherwise "inconclusive at this n". The secondary comparisons show the same numbers and no verdict word.

- D vs A (engine + new work): secondary, no verdict word. Effect +0.22, 95 % CI [0.00, +0.56], p = 0.500 (b = 2, c = 0), 9 pairs over 9 tasks.

## 2. Other paired rows (descriptive: secondary outcomes and kinds, no verdict words, no adjustment for many looks)

*Source: model `muse-spark-1.3`; run id `subset-1`.*

| comparison | outcome | kinds | pairs | tasks | b | c | p (exact McNemar) | effect | 95 % CI |
|---|---|---|---|---|---|---|---|---|---|
| D vs A (engine + new work) | later_kept | all | 9 | 9 | 0 | 0 | 1.000 (no discordant pairs) | 0.00 | [0.00, 0.00] |
| D vs A (engine + new work) | no break found (risk kinds) | all | 7 | 7 | 2 | 0 | 0.500 | +0.29 | [0.00, +0.71] |
| D vs A (engine + new work) | final_pass | natural-risk | 2 | 2 | 0 | 0 | 1.000 (no discordant pairs) | 0.00 | [0.00, 0.00] |
| D vs A (engine + new work) | final_pass | injected | 5 | 5 | 2 | 0 | 0.500 | +0.40 | [0.00, +0.80] |
| D vs A (engine + new work) | final_pass | control | 2 | 2 | 0 | 0 | 1.000 (no discordant pairs) | 0.00 | [0.00, 0.00] |

## 3. Per arm, all kinds

*Source: model `muse-spark-1.3`; run id `subset-1`.*

| metric | A | D |
|---|---|---|
| valid units (of units seen) | 9 of 9 | 9 of 9 |
| excluded: no valid attempt (3 failed, or one not re-run) / pending | 0 / 0 | 0 / 0 |
| attempts (invalid) | 9 (0) | 9 (0) |
| final_pass | 6/9 66.7 % [35.4-87.9] | 8/9 88.9 % [56.5-98.0] |
| later_kept | 9/9 100.0 % [70.1-100.0] | 9/9 100.0 % [70.1-100.0] |
| break_found (risk kinds only) | 3/7 42.9 % [15.8-75.0] | 1/7 14.3 % [2.6-51.3] |
| timeouts | 0/9 0.0 % [0.0-29.9], under about 3/9 (33.3 %) | 0/9 0.0 % [0.0-29.9], under about 3/9 (33.3 %) |
| gate blocks: total (per episode) | 0 (0) | 0 (0) |
| episodes with >= 1 gate block | 0/9 0.0 % [0.0-29.9], under about 3/9 (33.3 %) | 0/9 0.0 % [0.0-29.9], under about 3/9 (33.3 %) |
| false alarms: control episodes with >= 1 gate block | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) |
| rate_limited: total; episodes with >= 1 | 0; 0/9 0.0 % [0.0-29.9], under about 3/9 (33.3 %) | 0; 0/9 0.0 % [0.0-29.9], under about 3/9 (33.3 %) |
| hook_failures: total; episodes with >= 1 | 0; 0/9 0.0 % [0.0-29.9], under about 3/9 (33.3 %) | 0; 0/9 0.0 % [0.0-29.9], under about 3/9 (33.3 %) |
| minutes: median / total | 2.0 / 18.6 | 2.8 / 25.5 |
| turns: median / total | 6 / 54 | 6 / 54 |
| usd_hi: median / total | $1.872 / $18.701 | $2.152 / $19.501 |
| writer usd_hi total (writer calls), shown apart | $0.000 (0) | $0.952 (86) |
| steps: median / total | 33 / 332 | 37 / 340 |

k/n rate [Wilson 95 % interval, %]. A cell with 0 events adds "under about 3/n". medians and totals are over valid units. usd_hi is as recorded; writer usd_hi is shown apart and never added to it.

## 4. Per arm, by kind

### natural-risk

*Source: model `muse-spark-1.3`; run id `subset-1`.*

| metric | A | D |
|---|---|---|
| valid units (of units seen) | 2 of 2 | 2 of 2 |
| excluded: no valid attempt (3 failed, or one not re-run) / pending | 0 / 0 | 0 / 0 |
| attempts (invalid) | 2 (0) | 2 (0) |
| final_pass | 2/2 100.0 % [34.2-100.0] | 2/2 100.0 % [34.2-100.0] |
| later_kept | 2/2 100.0 % [34.2-100.0] | 2/2 100.0 % [34.2-100.0] |
| break_found (risk kinds only) | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) |
| timeouts | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) |
| gate blocks: total (per episode) | 0 (0) | 0 (0) |
| episodes with >= 1 gate block | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) |
| false alarms: control episodes with >= 1 gate block | n/a | n/a |
| rate_limited: total; episodes with >= 1 | 0; 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) | 0; 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) |
| hook_failures: total; episodes with >= 1 | 0; 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) | 0; 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) |
| minutes: median / total | 2.1 / 4.2 | 2.9 / 5.8 |
| turns: median / total | 6 / 12 | 6 / 12 |
| usd_hi: median / total | $2.200 / $4.399 | $2.229 / $4.458 |
| writer usd_hi total (writer calls), shown apart | $0.000 (0) | $0.221 (20) |
| steps: median / total | 38 / 76 | 39 / 78 |

### injected

*Source: model `muse-spark-1.3`; run id `subset-1`.*

| metric | A | D |
|---|---|---|
| valid units (of units seen) | 5 of 5 | 5 of 5 |
| excluded: no valid attempt (3 failed, or one not re-run) / pending | 0 / 0 | 0 / 0 |
| attempts (invalid) | 5 (0) | 5 (0) |
| final_pass | 2/5 40.0 % [11.8-76.9] | 4/5 80.0 % [37.6-96.4] |
| later_kept | 5/5 100.0 % [56.6-100.0] | 5/5 100.0 % [56.6-100.0] |
| break_found (risk kinds only) | 3/5 60.0 % [23.1-88.2] | 1/5 20.0 % [3.6-62.4] |
| timeouts | 0/5 0.0 % [0.0-43.4], under about 3/5 (60.0 %) | 0/5 0.0 % [0.0-43.4], under about 3/5 (60.0 %) |
| gate blocks: total (per episode) | 0 (0) | 0 (0) |
| episodes with >= 1 gate block | 0/5 0.0 % [0.0-43.4], under about 3/5 (60.0 %) | 0/5 0.0 % [0.0-43.4], under about 3/5 (60.0 %) |
| false alarms: control episodes with >= 1 gate block | n/a | n/a |
| rate_limited: total; episodes with >= 1 | 0; 0/5 0.0 % [0.0-43.4], under about 3/5 (60.0 %) | 0; 0/5 0.0 % [0.0-43.4], under about 3/5 (60.0 %) |
| hook_failures: total; episodes with >= 1 | 0; 0/5 0.0 % [0.0-43.4], under about 3/5 (60.0 %) | 0; 0/5 0.0 % [0.0-43.4], under about 3/5 (60.0 %) |
| minutes: median / total | 1.9 / 10.6 | 3.0 / 14.7 |
| turns: median / total | 6 / 30 | 6 / 30 |
| usd_hi: median / total | $1.872 / $10.675 | $2.152 / $11.007 |
| writer usd_hi total (writer calls), shown apart | $0.000 (0) | $0.544 (48) |
| steps: median / total | 35 / 191 | 37 / 192 |

### control

*Source: model `muse-spark-1.3`; run id `subset-1`.*

| metric | A | D |
|---|---|---|
| valid units (of units seen) | 2 of 2 | 2 of 2 |
| excluded: no valid attempt (3 failed, or one not re-run) / pending | 0 / 0 | 0 / 0 |
| attempts (invalid) | 2 (0) | 2 (0) |
| final_pass | 2/2 100.0 % [34.2-100.0] | 2/2 100.0 % [34.2-100.0] |
| later_kept | 2/2 100.0 % [34.2-100.0] | 2/2 100.0 % [34.2-100.0] |
| break_found (risk kinds only) | n/a | n/a |
| timeouts | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) |
| gate blocks: total (per episode) | 0 (0) | 0 (0) |
| episodes with >= 1 gate block | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) |
| false alarms: control episodes with >= 1 gate block | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) | 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) |
| rate_limited: total; episodes with >= 1 | 0; 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) | 0; 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) |
| hook_failures: total; episodes with >= 1 | 0; 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) | 0; 0/2 0.0 % [0.0-65.8], under about 3/2 (100.0 %) |
| minutes: median / total | 1.9 / 3.8 | 2.5 / 5.0 |
| turns: median / total | 6 / 12 | 6 / 12 |
| usd_hi: median / total | $1.813 / $3.626 | $2.018 / $4.035 |
| writer usd_hi total (writer calls), shown apart | $0.000 (0) | $0.187 (18) |
| steps: median / total | 32.5 / 65 | 35 / 70 |

## 5. Attempts and exclusions, per arm

- A: 9 units, 9 valid, 0 excluded, no valid attempt (3 failed attempts, or one not re-run), 0 pending; 0 of 9 attempts invalid. Invalid attempts: none. Excluded units' attempts: none. Network lines outside api.meta.ai:443 (egress.other; recorded, never a reason to exclude): none.
- D: 9 units, 9 valid, 0 excluded, no valid attempt (3 failed attempts, or one not re-run), 0 pending; 0 of 9 attempts invalid. Invalid attempts: none. Excluded units' attempts: none. Network lines outside api.meta.ai:443 (egress.other; recorded, never a reason to exclude): none.

## 6. Per task (pairs only)

D vs A (engine + new work)

*Source: model `muse-spark-1.3`; run id `subset-1`.*

| task | kind | torpor_should_help | pairs | D passes | A passes | b | c |
|---|---|---|---|---|---|---|---|
| notes-ctrl | control | no | 1 | 1/1 | 1/1 | 0 | 0 |
| notes-dup | injected | no | 1 | 0/1 | 0/1 | 0 | 0 |
| notes-logout | injected | yes | 1 | 1/1 | 1/1 | 0 | 0 |
| shop-clear | injected | yes | 1 | 1/1 | 1/1 | 0 | 0 |
| shop-coupon | natural-risk | yes | 1 | 1/1 | 1/1 | 0 | 0 |
| shop-merge | injected | yes | 1 | 1/1 | 0/1 | 1 | 0 |
| todo-ctrl | control | no | 1 | 1/1 | 1/1 | 0 | 0 |
| todo-persist | injected | yes | 1 | 1/1 | 0/1 | 1 | 0 |
| todo-tags | natural-risk | maybe | 1 | 1/1 | 1/1 | 0 | 0 |
