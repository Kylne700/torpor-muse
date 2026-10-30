# Eval results

*Source: model `muse-spark-1.3-contributor`; run id `main-1`.*

Rows read: 168. Units (task x arm x rep): 168; valid 168, excluded, no valid attempt (3 failed attempts, or one not re-run) 0, pending 0.
Analysis: `eval/analysis/analyze.mjs`, sha256 `63939820e82c3ce52a4e361adee8d02d4724cdb8a011e0f61dd5589724a5e0ea`; bootstrap seed 1, 10000 resamples; Wilson z = 1.959964.

## 1. Pre-registered comparisons (final_pass, all kinds pooled)

*Source: model `muse-spark-1.3-contributor`; run id `main-1`.*

| comparison | pairs | tasks | first arm passes | second arm passes | b | c | p (exact McNemar) | effect | 95 % CI (cluster bootstrap) | verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| D vs C (primary) | 42 | 21 | 34/42 | 32/42 | 3 | 1 | 0.625 | +0.05 | [-0.05, +0.14] | inconclusive at this n |
| D vs A (engine + new work) | 42 | 21 | 34/42 | 34/42 | 2 | 2 | 1.000 | 0.00 | [-0.10, +0.14] | none (secondary) |
| B vs A (notes file vs vanilla) | 42 | 21 | 36/42 | 34/42 | 3 | 1 | 0.625 | +0.05 | [-0.05, +0.19] | none (secondary) |

b = pairs where the first-named arm passed and the second failed; c = the reverse; effect = (b - c) / pairs, the difference in pass rate.
p = exact two-sided McNemar on b + c. CI = 95 % percentile interval of a cluster bootstrap that resamples TASKS with replacement.
Verdict words are fixed in advance and only the primary comparison (D vs C) gets one: "D better than C" only if p < 0.05 AND the whole interval is above 0; otherwise "inconclusive at this n". The secondary comparisons show the same numbers and no verdict word.

- D vs C (primary): inconclusive at this n. Effect +0.05, 95 % CI [-0.05, +0.14], p = 0.625 (b = 3, c = 1), 42 pairs over 21 tasks.
- D vs A (engine + new work): secondary, no verdict word. Effect 0.00, 95 % CI [-0.10, +0.14], p = 1.000 (b = 2, c = 2), 42 pairs over 21 tasks.
- B vs A (notes file vs vanilla): secondary, no verdict word. Effect +0.05, 95 % CI [-0.05, +0.19], p = 0.625 (b = 3, c = 1), 42 pairs over 21 tasks.

## 2. Other paired rows (descriptive: secondary outcomes and kinds, no verdict words, no adjustment for many looks)

*Source: model `muse-spark-1.3-contributor`; run id `main-1`.*

| comparison | outcome | kinds | pairs | tasks | b | c | p (exact McNemar) | effect | 95 % CI |
|---|---|---|---|---|---|---|---|---|---|
| D vs C (primary) | later_kept | all | 42 | 21 | 0 | 0 | 1.000 (no discordant pairs) | 0.00 | [0.00, 0.00] |
| D vs C (primary) | no break found (risk kinds) | all | 36 | 18 | 3 | 1 | 0.625 | +0.06 | [-0.06, +0.17] |
| D vs C (primary) | final_pass | natural-risk | 18 | 9 | 0 | 0 | 1.000 (no discordant pairs) | 0.00 | [0.00, 0.00] |
| D vs C (primary) | final_pass | injected | 18 | 9 | 3 | 1 | 0.625 | +0.11 | [-0.11, +0.33] |
| D vs C (primary) | final_pass | control | 6 | 3 | 0 | 0 | 1.000 (no discordant pairs) | 0.00 | [0.00, 0.00] |
| D vs A (engine + new work) | later_kept | all | 42 | 21 | 0 | 0 | 1.000 (no discordant pairs) | 0.00 | [0.00, 0.00] |
| D vs A (engine + new work) | no break found (risk kinds) | all | 36 | 18 | 2 | 2 | 1.000 | 0.00 | [-0.11, +0.14] |
| D vs A (engine + new work) | final_pass | natural-risk | 18 | 9 | 0 | 0 | 1.000 (no discordant pairs) | 0.00 | [0.00, 0.00] |
| D vs A (engine + new work) | final_pass | injected | 18 | 9 | 2 | 2 | 1.000 | 0.00 | [-0.22, +0.28] |
| D vs A (engine + new work) | final_pass | control | 6 | 3 | 0 | 0 | 1.000 (no discordant pairs) | 0.00 | [0.00, 0.00] |
| B vs A (notes file vs vanilla) | later_kept | all | 42 | 21 | 0 | 1 | 1.000 | -0.02 | [-0.07, 0.00] |
| B vs A (notes file vs vanilla) | no break found (risk kinds) | all | 36 | 18 | 3 | 0 | 0.250 | +0.08 | [0.00, +0.22] |
| B vs A (notes file vs vanilla) | final_pass | natural-risk | 18 | 9 | 0 | 0 | 1.000 (no discordant pairs) | 0.00 | [0.00, 0.00] |
| B vs A (notes file vs vanilla) | final_pass | injected | 18 | 9 | 3 | 1 | 0.625 | +0.11 | [-0.11, +0.39] |
| B vs A (notes file vs vanilla) | final_pass | control | 6 | 3 | 0 | 0 | 1.000 (no discordant pairs) | 0.00 | [0.00, 0.00] |

## 3. Per arm, all kinds

*Source: model `muse-spark-1.3-contributor`; run id `main-1`.*

| metric | A | B | C | D |
|---|---|---|---|---|
| valid units (of units seen) | 42 of 42 | 42 of 42 | 42 of 42 | 42 of 42 |
| excluded: no valid attempt (3 failed, or one not re-run) / pending | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| attempts (invalid) | 42 (0) | 42 (0) | 42 (0) | 42 (0) |
| final_pass | 34/42 81.0 % [66.7-90.0] | 36/42 85.7 % [72.2-93.3] | 32/42 76.2 % [61.5-86.5] | 34/42 81.0 % [66.7-90.0] |
| later_kept | 42/42 100.0 % [91.6-100.0] | 41/42 97.6 % [87.7-99.6] | 42/42 100.0 % [91.6-100.0] | 42/42 100.0 % [91.6-100.0] |
| break_found (risk kinds only) | 8/36 22.2 % [11.7-38.1] | 5/36 13.9 % [6.1-28.7] | 10/36 27.8 % [15.8-44.0] | 8/36 22.2 % [11.7-38.1] |
| timeouts | 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) | 1/42 2.4 % [0.4-12.3] | 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) | 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) |
| gate blocks: total (per episode) | 0 (0) | 0 (0) | 0 (0) | 0 (0) |
| episodes with >= 1 gate block | 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) | 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) | 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) | 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) |
| false alarms: control episodes with >= 1 gate block | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) |
| rate_limited: total; episodes with >= 1 | 0; 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) | 0; 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) | 0; 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) | 0; 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) |
| hook_failures: total; episodes with >= 1 | 0; 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) | 0; 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) | 0; 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) | 0; 0/42 0.0 % [0.0-8.4], under about 3/42 (7.1 %) |
| minutes: median / total | 2.6 / 108.0 | 2.9 / 129.2 | 2.7 / 120.5 | 3.2 / 137.6 |
| turns: median / total | 6 / 252 | 6 / 249 | 6 / 252 | 6 / 252 |
| usd_hi: median / total | $0.140 / $6.113 | $0.161 / $7.105 | $0.159 / $6.788 | $0.163 / $6.970 |
| writer usd_hi total (writer calls), shown apart | $0.000 (0) | $0.000 (0) | $0.000 (0) | $4.537 (404) |
| steps: median / total | 34 / 1471 | 38.5 / 1642 | 38 / 1603 | 37.5 / 1634 |

k/n rate [Wilson 95 % interval, %]. A cell with 0 events adds "under about 3/n". medians and totals are over valid units. usd_hi is as recorded; writer usd_hi is shown apart and never added to it.

## 4. Per arm, by kind

### natural-risk

*Source: model `muse-spark-1.3-contributor`; run id `main-1`.*

| metric | A | B | C | D |
|---|---|---|---|---|
| valid units (of units seen) | 18 of 18 | 18 of 18 | 18 of 18 | 18 of 18 |
| excluded: no valid attempt (3 failed, or one not re-run) / pending | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| attempts (invalid) | 18 (0) | 18 (0) | 18 (0) | 18 (0) |
| final_pass | 18/18 100.0 % [82.4-100.0] | 18/18 100.0 % [82.4-100.0] | 18/18 100.0 % [82.4-100.0] | 18/18 100.0 % [82.4-100.0] |
| later_kept | 18/18 100.0 % [82.4-100.0] | 18/18 100.0 % [82.4-100.0] | 18/18 100.0 % [82.4-100.0] | 18/18 100.0 % [82.4-100.0] |
| break_found (risk kinds only) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) |
| timeouts | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) |
| gate blocks: total (per episode) | 0 (0) | 0 (0) | 0 (0) | 0 (0) |
| episodes with >= 1 gate block | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) |
| false alarms: control episodes with >= 1 gate block | n/a | n/a | n/a | n/a |
| rate_limited: total; episodes with >= 1 | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) |
| hook_failures: total; episodes with >= 1 | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) |
| minutes: median / total | 2.9 / 50.3 | 2.8 / 51.1 | 2.9 / 55.3 | 3.4 / 60.4 |
| turns: median / total | 6 / 108 | 6 / 108 | 6 / 108 | 6 / 108 |
| usd_hi: median / total | $0.145 / $2.710 | $0.154 / $2.860 | $0.168 / $3.081 | $0.162 / $3.050 |
| writer usd_hi total (writer calls), shown apart | $0.000 (0) | $0.000 (0) | $0.000 (0) | $1.957 (175) |
| steps: median / total | 35 / 642 | 36 / 666 | 38.5 / 708 | 37.5 / 706 |

### injected

*Source: model `muse-spark-1.3-contributor`; run id `main-1`.*

| metric | A | B | C | D |
|---|---|---|---|---|
| valid units (of units seen) | 18 of 18 | 18 of 18 | 18 of 18 | 18 of 18 |
| excluded: no valid attempt (3 failed, or one not re-run) / pending | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| attempts (invalid) | 18 (0) | 18 (0) | 18 (0) | 18 (0) |
| final_pass | 10/18 55.6 % [33.7-75.4] | 12/18 66.7 % [43.7-83.7] | 8/18 44.4 % [24.6-66.3] | 10/18 55.6 % [33.7-75.4] |
| later_kept | 18/18 100.0 % [82.4-100.0] | 17/18 94.4 % [74.2-99.0] | 18/18 100.0 % [82.4-100.0] | 18/18 100.0 % [82.4-100.0] |
| break_found (risk kinds only) | 8/18 44.4 % [24.6-66.3] | 5/18 27.8 % [12.5-50.9] | 10/18 55.6 % [33.7-75.4] | 8/18 44.4 % [24.6-66.3] |
| timeouts | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 1/18 5.6 % [1.0-25.8] | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) |
| gate blocks: total (per episode) | 0 (0) | 0 (0) | 0 (0) | 0 (0) |
| episodes with >= 1 gate block | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) |
| false alarms: control episodes with >= 1 gate block | n/a | n/a | n/a | n/a |
| rate_limited: total; episodes with >= 1 | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) |
| hook_failures: total; episodes with >= 1 | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) | 0; 0/18 0.0 % [0.0-17.6], under about 3/18 (16.7 %) |
| minutes: median / total | 2.4 / 44.1 | 3.2 / 63.7 | 2.8 / 51.3 | 3.1 / 57.1 |
| turns: median / total | 6 / 108 | 6 / 105 | 6 / 108 | 6 / 108 |
| usd_hi: median / total | $0.136 / $2.598 | $0.174 / $3.250 | $0.163 / $2.844 | $0.163 / $2.958 |
| writer usd_hi total (writer calls), shown apart | $0.000 (0) | $0.000 (0) | $0.000 (0) | $1.868 (169) |
| steps: median / total | 33 / 629 | 40.5 / 745 | 39 / 682 | 37.5 / 699 |

### control

*Source: model `muse-spark-1.3-contributor`; run id `main-1`.*

| metric | A | B | C | D |
|---|---|---|---|---|
| valid units (of units seen) | 6 of 6 | 6 of 6 | 6 of 6 | 6 of 6 |
| excluded: no valid attempt (3 failed, or one not re-run) / pending | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| attempts (invalid) | 6 (0) | 6 (0) | 6 (0) | 6 (0) |
| final_pass | 6/6 100.0 % [61.0-100.0] | 6/6 100.0 % [61.0-100.0] | 6/6 100.0 % [61.0-100.0] | 6/6 100.0 % [61.0-100.0] |
| later_kept | 6/6 100.0 % [61.0-100.0] | 6/6 100.0 % [61.0-100.0] | 6/6 100.0 % [61.0-100.0] | 6/6 100.0 % [61.0-100.0] |
| break_found (risk kinds only) | n/a | n/a | n/a | n/a |
| timeouts | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) |
| gate blocks: total (per episode) | 0 (0) | 0 (0) | 0 (0) | 0 (0) |
| episodes with >= 1 gate block | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) |
| false alarms: control episodes with >= 1 gate block | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) |
| rate_limited: total; episodes with >= 1 | 0; 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0; 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0; 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0; 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) |
| hook_failures: total; episodes with >= 1 | 0; 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0; 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0; 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) | 0; 0/6 0.0 % [0.0-39.0], under about 3/6 (50.0 %) |
| minutes: median / total | 2.4 / 13.6 | 2.3 / 14.5 | 2.2 / 13.9 | 3.4 / 20.1 |
| turns: median / total | 6 / 36 | 6 / 36 | 6 / 36 | 6 / 36 |
| usd_hi: median / total | $0.133 / $0.806 | $0.160 / $0.995 | $0.144 / $0.862 | $0.163 / $0.961 |
| writer usd_hi total (writer calls), shown apart | $0.000 (0) | $0.000 (0) | $0.000 (0) | $0.712 (60) |
| steps: median / total | 33 / 200 | 38 / 231 | 35.5 / 213 | 37.5 / 229 |

## 5. Attempts and exclusions, per arm

- A: 42 units, 42 valid, 0 excluded, no valid attempt (3 failed attempts, or one not re-run), 0 pending; 0 of 42 attempts invalid. Invalid attempts: none. Excluded units' attempts: none. Network lines outside api.meta.ai:443 (egress.other; recorded, never a reason to exclude): none.
- B: 42 units, 42 valid, 0 excluded, no valid attempt (3 failed attempts, or one not re-run), 0 pending; 0 of 42 attempts invalid. Invalid attempts: none. Excluded units' attempts: none. Network lines outside api.meta.ai:443 (egress.other; recorded, never a reason to exclude): none.
- C: 42 units, 42 valid, 0 excluded, no valid attempt (3 failed attempts, or one not re-run), 0 pending; 0 of 42 attempts invalid. Invalid attempts: none. Excluded units' attempts: none. Network lines outside api.meta.ai:443 (egress.other; recorded, never a reason to exclude): none.
- D: 42 units, 42 valid, 0 excluded, no valid attempt (3 failed attempts, or one not re-run), 0 pending; 0 of 42 attempts invalid. Invalid attempts: none. Excluded units' attempts: none. Network lines outside api.meta.ai:443 (egress.other; recorded, never a reason to exclude): none.

## 6. Per task (pairs only)

D vs C (primary)

*Source: model `muse-spark-1.3-contributor`; run id `main-1`.*

| task | kind | torpor_should_help | pairs | D passes | C passes | b | c |
|---|---|---|---|---|---|---|---|
| notes-cookie | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| notes-ctrl | control | no | 2 | 2/2 | 2/2 | 0 | 0 |
| notes-dup | injected | no | 2 | 0/2 | 0/2 | 0 | 0 |
| notes-hash | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| notes-logout | injected | yes | 2 | 2/2 | 1/2 | 1 | 0 |
| notes-persist | injected | yes | 2 | 0/2 | 0/2 | 0 | 0 |
| notes-ratelimit | natural-risk | no | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-clear | injected | yes | 2 | 2/2 | 1/2 | 1 | 0 |
| shop-coupon | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-ctrl | control | no | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-customer | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-merge | injected | yes | 2 | 2/2 | 1/2 | 1 | 0 |
| shop-remove | injected | no | 2 | 0/2 | 0/2 | 0 | 0 |
| shop-vat | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-ctrl | control | no | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-delete | injected | no | 2 | 1/2 | 2/2 | 0 | 1 |
| todo-logout | injected | yes | 2 | 1/2 | 1/2 | 0 | 0 |
| todo-persist | injected | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-router | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-tags | natural-risk | maybe | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-validate | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |

D vs A (engine + new work)

*Source: model `muse-spark-1.3-contributor`; run id `main-1`.*

| task | kind | torpor_should_help | pairs | D passes | A passes | b | c |
|---|---|---|---|---|---|---|---|
| notes-cookie | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| notes-ctrl | control | no | 2 | 2/2 | 2/2 | 0 | 0 |
| notes-dup | injected | no | 2 | 0/2 | 0/2 | 0 | 0 |
| notes-hash | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| notes-logout | injected | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| notes-persist | injected | yes | 2 | 0/2 | 0/2 | 0 | 0 |
| notes-ratelimit | natural-risk | no | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-clear | injected | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-coupon | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-ctrl | control | no | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-customer | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-merge | injected | yes | 2 | 2/2 | 0/2 | 2 | 0 |
| shop-remove | injected | no | 2 | 0/2 | 0/2 | 0 | 0 |
| shop-vat | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-ctrl | control | no | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-delete | injected | no | 2 | 1/2 | 2/2 | 0 | 1 |
| todo-logout | injected | yes | 2 | 1/2 | 2/2 | 0 | 1 |
| todo-persist | injected | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-router | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-tags | natural-risk | maybe | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-validate | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |

B vs A (notes file vs vanilla)

*Source: model `muse-spark-1.3-contributor`; run id `main-1`.*

| task | kind | torpor_should_help | pairs | B passes | A passes | b | c |
|---|---|---|---|---|---|---|---|
| notes-cookie | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| notes-ctrl | control | no | 2 | 2/2 | 2/2 | 0 | 0 |
| notes-dup | injected | no | 2 | 0/2 | 0/2 | 0 | 0 |
| notes-hash | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| notes-logout | injected | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| notes-persist | injected | yes | 2 | 1/2 | 0/2 | 1 | 0 |
| notes-ratelimit | natural-risk | no | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-clear | injected | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-coupon | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-ctrl | control | no | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-customer | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| shop-merge | injected | yes | 2 | 2/2 | 0/2 | 2 | 0 |
| shop-remove | injected | no | 2 | 0/2 | 0/2 | 0 | 0 |
| shop-vat | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-ctrl | control | no | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-delete | injected | no | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-logout | injected | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-persist | injected | yes | 2 | 1/2 | 2/2 | 0 | 1 |
| todo-router | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-tags | natural-risk | maybe | 2 | 2/2 | 2/2 | 0 | 0 |
| todo-validate | natural-risk | yes | 2 | 2/2 | 2/2 | 0 | 0 |
