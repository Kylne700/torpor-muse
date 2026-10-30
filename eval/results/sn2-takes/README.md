# sn2-takes: the 16 takes of SN2

One JSON file per take (`new-V0-take1.json` to `base-V0-take8.json`) behind the section [SN2 of e2e/PROOF.md](../../../e2e/PROOF.md#sn2-what-the-user-sees-when-a-check-fails-on-muse-code-1414). SN2 is not part of the pre-registered study ([main-1](../main-1/README.md)): it was planned in writing before the takes, 16 takes, one run per take, no take run again or replaced. 8 takes per build allow no rate.

## What the files say about the check-writer

On Muse Code 1.4.14 with the pinned engine torpor 2.3.1, in 16 keyed takes of the V0 script (8 on the `new` build b9871b5, 8 on `base`, main 56a6a11), the check-writer saved the check of the claim prompt in 16 of 16 takes. Its run took 8.8 to 21.1 s (median 12.4 s) and cost USD 0.008072 to 0.018376 (median 0.0109505), Torpor's estimate from token counts, not Meta's bill. Two calls in 15 takes, three in 1.

The figures come from `writer_result` of each file (the check-writer's record right after the claim prompt: "login accepts the demo user"). `node eval/results/sn2-takes/summary.mjs` prints them from the 16 files, with no dependency:

| measure | value |
|---|---|
| takes read | 16 (new 8, base 8) |
| started, last ended | 2026-10-29T19:46:34Z, 2026-10-29T20:13:56Z |
| Muse Code, mode | 1.4.14, keyed |
| takes the recorder finished without an error (`ok`) | 16 |
| takes in which the recorder saw the break in the app (`facts.break_seen`) | 13 |
| check saved (`writer_result.saved`) | 16 of 16 |
| time of the check-writer's run, ms | min 8783, median 12399.5, max 21135 |
| time, s | min 8.8, median 12.4, max 21.1 |
| cost, USD (Torpor's estimate) | min 0.008072, median 0.0109505, max 0.018376 |
| calls of that run | 2 in 15, 3 in 1 |
| model of the check-writer | muse-spark-1.3 |

`writer_result_last` is the record at the end of the take, which belongs to the last prompt (the break prompt, not a claim): it saved nothing in 16 of 16 takes, and its times and costs are other numbers. `summary.mjs` prints it apart; it is not the figure above.

## What is in a file

`name`, `variant` (V0), `arm` (`new` or `base`), `seg`, `stub`, `echo_delay_ms`, `mode`; `started_at`, `ended_at` (UTC); `markers` (marks of the recorder, in seconds from the start of the take); `facts` (what the recorder counted and read: `break_seen`, `broken_at_s`, `fixed_at_s`, `check_all`, `promise_list`, ...); `muse_banner`; `engine_sha256` (equal to the linux-x64 asset in [engine.lock](../../../engine.lock)) and `adapter_sha256` (the adapter binary used; it differs between `new` and `base`); `clone_sha` (the commit of the clone of main that supplied the recorder's helper code, the same in every file, so the two builds are told apart by `adapter_sha256`, not by this field); `writer_result` and `writer_result_last`; `spend` (`lo` and `hi`, Torpor's low and high estimate in USD for the take, and one row per call: Muse Code's model calls and the check-writer's calls); `why_stopped`; `ok`.

Values are as recorded. Left out: `birth_ms`, `session_logs` and `cast` (they point at Muse session logs and screen recordings that are not published) and `session` inside both writer records (a Muse session id). The Muse session logs and the screen recordings of the takes are not in this folder.
