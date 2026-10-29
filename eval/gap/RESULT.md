# S3g result: the writer names only what the developer said works

Made by `node eval/gap/run.mjs report eval/gap/rows.jsonl` from the recorded runs. The bars were fixed in PREREG.md before the first keyed call. Nothing here was re-run.

Model muse-spark-1.3; engine torpor 2.3.1; harness commit c425fbbb97401700d541dc19d53805f950848703; BASE binary sha256 d1b8d63b65c65ed0, HEAD binary sha256 b846b54e8704aa41; upstream api; 2026-10-29T03:58:04.590Z to 2026-10-29T04:05:15.260Z.

## Pass lines

| bar | result | met |
|---|---|---|
| HEAD junk at most 1 of 25 | 0 of 25 | yes |
| HEAD kept at least BASE kept minus 1 (of 10) | HEAD 10 of 10, BASE 7 of 10 | yes |
| HEAD saved at least BASE saved minus 2 (of 25) | HEAD 20 of 25, BASE 12 of 25 | yes |

## Tallies

`note` = the check was saved and its reason is not empty (the case that puts a note in the AI's context). It is descriptive, not a pass line.

| input | arm | runs | verdict check | saved | note | junk | kept |
|---|---|---|---|---|---|---|---|
| A1 | BASE | 5 | 5 | 5 | 0 | 0 | - |
| A1 | HEAD | 5 | 5 | 5 | 0 | 0 | - |
| A2 | BASE | 5 | 0 | 0 | 0 | 0 | - |
| A2 | HEAD | 5 | 0 | 0 | 0 | 0 | - |
| A3 | BASE | 5 | 0 | 0 | 0 | 5 | - |
| A3 | HEAD | 5 | 5 | 5 | 0 | 0 | - |
| C1 | BASE | 5 | 5 | 5 | 5 | 1 | 5 of 5 |
| C1 | HEAD | 5 | 5 | 5 | 5 | 0 | 5 of 5 |
| C2 | BASE | 5 | 2 | 2 | 2 | 3 | 2 of 5 |
| C2 | HEAD | 5 | 5 | 5 | 5 | 0 | 5 of 5 |
| all | BASE | 25 | 12 | 12 | 7 | 9 | 7 of 10 |
| all | HEAD | 25 | 20 | 20 | 10 | 0 | 10 of 10 |

## Every run

Sorted by input, arm and repetition. A reason scored from the last writer answer that parsed; `none` = no usable answer (scores 0 for junk, kept and saved).

| input | arm | rep | verdict | saved | junk | kept | engine said | reason |
|---|---|---|---|---|---|---|---|---|
| A1 | BASE | 1 | check | 1 | 0 | - | advisory | (empty) |
| A1 | BASE | 2 | check | 1 | 0 | - | advisory | (empty) |
| A1 | BASE | 3 | check | 1 | 0 | - | advisory | (empty) |
| A1 | BASE | 4 | check | 1 | 0 | - | advisory | (empty) |
| A1 | BASE | 5 | check | 1 | 0 | - | advisory | (empty) |
| A1 | HEAD | 1 | check | 1 | 0 | - | advisory | (empty) |
| A1 | HEAD | 2 | check | 1 | 0 | - | advisory | (empty) |
| A1 | HEAD | 3 | check | 1 | 0 | - | proven | (empty) |
| A1 | HEAD | 4 | check | 1 | 0 | - | advisory | (empty) |
| A1 | HEAD | 5 | check | 1 | 0 | - | advisory | (empty) |
| A2 | BASE | 1 | cannot_check | 0 | 0 | - | advisory | The visible server.js code shows no health route path to request. |
| A2 | BASE | 2 | cannot_check | 0 | 0 | - | advisory | No health route path appears in the shown server.js code so it cannot be requested. |
| A2 | BASE | 3 | cannot_check | 0 | 0 | - | advisory | No health route path appears in the shown server code so it cannot be checked. |
| A2 | BASE | 4 | cannot_check | 0 | 0 | - | advisory | No health route path appears in the visible server.js excerpt. |
| A2 | BASE | 5 | cannot_check | 0 | 0 | - | advisory | The truncated server.js does not show a health route path to test. |
| A2 | HEAD | 1 | cannot_check | 0 | 0 | - | advisory | The visible server.js shows no health route to test. |
| A2 | HEAD | 2 | cannot_check | 0 | 0 | - | advisory | No health route path appears in the visible server code to test. |
| A2 | HEAD | 3 | cannot_check | 0 | 0 | - | advisory | The truncated server file shows no health route path to request. |
| A2 | HEAD | 4 | cannot_check | 0 | 0 | - | advisory | The visible code does not show any health route path to request. |
| A2 | HEAD | 5 | cannot_check | 0 | 0 | - | advisory | No health route path appears in the shown server code so it cannot be requested. |
| A3 | BASE | 1 | not_a_claim | 0 | 1 | - | advisory | The message requests adding a Forgot password link rather than stating that new behavior works. |
| A3 | BASE | 2 | not_a_claim | 0 | 1 | - | advisory | The message requests adding a Forgot password link rather than claiming something already works. |
| A3 | BASE | 3 | not_a_claim | 0 | 1 | - | advisory | The message is a request to add a new link, not a statement that something already works. |
| A3 | BASE | 4 | not_a_claim | 0 | 1 | - | advisory | The message is a request to add a Forgot password link, not a claim that something already works. |
| A3 | BASE | 5 | not_a_claim | 0 | 1 | - | advisory | The message requests a new Forgot password link rather than claiming something already works. |
| A3 | HEAD | 1 | check | 1 | 0 | - | advisory | (empty) |
| A3 | HEAD | 2 | check | 1 | 0 | - | advisory | (empty) |
| A3 | HEAD | 3 | check | 1 | 0 | - | advisory | (empty) |
| A3 | HEAD | 4 | check | 1 | 0 | - | advisory | (empty) |
| A3 | HEAD | 5 | check | 1 | 0 | - | advisory | (empty) |
| C1 | BASE | 1 | check | 1 | 0 | 1 | advisory | Login persistence across a server restart cannot be tested because these steps run without restarting the server. |
| C1 | BASE | 2 | check | 1 | 0 | 1 | advisory | Persistence across a server restart cannot be tested because these steps cannot restart the server. |
| C1 | BASE | 3 | check | 1 | 1 | 1 | advisory | Persistence after a server restart and the requested Show password checkbox cannot be verified by these HTTP steps. |
| C1 | BASE | 4 | check | 1 | 0 | 1 | advisory | Session persistence across server restarts cannot be tested without restarting the server. |
| C1 | BASE | 5 | check | 1 | 0 | 1 | advisory | Session persistence across a server restart cannot be tested without restarting the server. |
| C1 | HEAD | 1 | check | 1 | 0 | 1 | advisory | Persistence across a server restart cannot be tested without restarting the server. |
| C1 | HEAD | 2 | check | 1 | 0 | 1 | advisory | Login persistence across a server restart cannot be tested without restarting the server. |
| C1 | HEAD | 3 | check | 1 | 0 | 1 | advisory | The check cannot verify that sessions survive server restarts. |
| C1 | HEAD | 4 | check | 1 | 0 | 1 | advisory | The check does not restart the server, so session survival after restarts cannot be verified. |
| C1 | HEAD | 5 | check | 1 | 0 | 1 | advisory | Persistence across restarts cannot be checked here because it requires restarting the server. |
| C2 | BASE | 1 | check | 1 | 0 | 1 | advisory | Session expiry after 30 minutes cannot be tested without waiting for time to pass. |
| C2 | BASE | 2 | not_a_claim | 0 | 1 | 0 | advisory | The message asks for a new Forgot password link instead of stating that new work now functions. |
| C2 | BASE | 3 | not_a_claim | 0 | 1 | 0 | advisory | The message requests adding a new link rather than stating that implemented work now works. |
| C2 | BASE | 4 | check | 1 | 0 | 1 | advisory | Session expiry after 30 minutes cannot be checked without waiting for time to pass. |
| C2 | BASE | 5 | not_a_claim | 0 | 1 | 0 | advisory | The message is a request to add a Forgot password link, not a report that something works. |
| C2 | HEAD | 1 | check | 1 | 0 | 1 | advisory | Session expiry after 30 minutes cannot be checked without waiting for time to pass. |
| C2 | HEAD | 2 | check | 1 | 0 | 1 | advisory | Cannot test that the session ends after 30 minutes without waiting for time to pass. |
| C2 | HEAD | 3 | check | 1 | 0 | 1 | advisory | The 30-minute session expiry cannot be checked without waiting for time to pass. |
| C2 | HEAD | 4 | check | 1 | 0 | 1 | advisory | Cannot verify the session ends after 30 minutes without waiting for time to pass. |
| C2 | HEAD | 5 | check | 1 | 0 | 1 | advisory | The 30-minute session expiry cannot be tested without waiting for time to pass. |

Calls: 0.4160 USD lo and 0.7381 USD hi over 50 writer calls in the runs above (exact sums; the spend ledger adds its rounded rows and is the record of spend; cap 3.00 hi).

## Notes (written by hand after the run)

Everything above this heading is the unchanged output of `report`. Nothing below re-scores a run, and nothing was re-run.

- Pre-registration. `eval/gap/PREREG.md` and `eval/gap/run.mjs` were pushed in commit c425fbbb97401700d541dc19d53805f950848703. GitHub's activity list shows the branch `gap-claims-only` created with that tip at 2026-10-29T03:57:44Z (the local `date -u` right after the push read 03:57:47Z). The sitting started at 03:58:04Z; the first response of the API is row 1 of the spend ledger, 03:58:10Z. PREREG.md has not changed since that push.
- The sitting. One sitting, 50 of 50 runs, no stop and no resume (`resumes` is 0 in `rows.jsonl`; the stand-in stops the sitting on any answer other than HTTP 200, and none came). Each run made one writer call and the adapter needed one attempt in all 50, so the ledger has 50 rows.
- Result. All three bars are met: HEAD junk 0 of 25 (BASE 9), HEAD kept 10 of 10 (BASE 7), HEAD saved 20 of 25 (BASE 12). BASE junk is not 0, so the "bug not reproduced" line does not apply. No significance test was planned or run; there are 5 runs per input and arm.
- What the BASE junk is. 8 of the 9 BASE junk reasons are in runs where the writer answered `not_a_claim` (A3 5 of 5, C2 3 of 5). There the reason is the writer's explanation of why it wrote no check (A3 BASE rep 5: "The message requests a new Forgot password link rather than claiming something already works."). It names the request, so the key words fire, but no check is saved and nothing reaches the AI (note 0). Only 1 of the 9 (C1 BASE rep 3, a saved check whose note names the restart and also "the requested Show password checkbox") is the failure seen in the e2e L4c: a note in the AI's context that names the request. The exact L4c case, a clean claim plus a request (A1 and A3), did not reproduce at BASE: A1 gave 5 checks with no note, A3 gave 5 `not_a_claim`.
- What HEAD changed in these runs. No reason names the request (0 of 25), and all 10 notes of C1 and C2 name the real gap (a restart; waiting 30 minutes). The larger difference is the verdict on A3 and C2: BASE answered `not_a_claim` in 8 of those 10 runs (no check saved for a claim the developer made), HEAD wrote a check in 10 of 10. That is the whole rise in `saved` (12 to 20 of 25; A1, A2 and C1 are equal in both arms). PREREG did not predict it. The arms differ only in the one prompt line, but 5 runs per cell do not say how much of the shift is the sentence and how much is the model's own variation.
- A2 carried no information. Both arms answered `cannot_check` in all 10 runs, and the reasons say the visible `server.js` shows no health route. The demo's `server.js` is 6775 bytes before the HEALTH edit, the excerpt the writer gets is cut at 6000 bytes per file (`FILE_CAP` in `src/suggest/repo/excerpt.rs`), and the HEALTH edit sits near the end of the file, past the cut. A script applying the line cap and the file cap of `read_capped` (not its redaction step) to the A2 app state puts `api/health` at byte 6492 of 6886, so it is cut off; no request body was kept to check it against. This is a flaw in the input I pre-registered, not an effect of the change. A2 counts in the totals as pre-registered (0 saved and 0 junk in both arms) and moves no difference between the arms.
- Reading the tables. The column "engine said" is the `outcome` field of the adapter's result file: `proven` once, `advisory` (a guard only, no earlier version fails it) for the other 31 saved checks. The 18 runs that saved nothing (8 `not_a_claim`, 10 `cannot_check`; the adapter exited 3) also read `advisory` there, which carries no meaning. junk and kept are key-word matches on the reason of the last parsed writer answer, whatever its verdict (as PREREG says); the `note` column and the printed reasons are there so a reader can judge.
- Not measured here. The first question was canned (`claim`), so the effect with the real first question is not measured; the history is made by the engine's hooks, not by a live AI session.
- Spend. `scratch/torpor-s3/spend-S3g.md` (in the project root, outside this repository): 50 calls, 0.4159 USD lo and 0.7385 USD hi against the 3.00 USD cap (sums of the rounded rows; the exact sums are in the Calls line above). The Meta balance was not read by this stage.
- Raw data. `eval/gap/rows.jsonl`, one line of metadata and one line per run, written by the harness.
