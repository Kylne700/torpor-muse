# S3x result: the writer sees the code a claim is about, also far into a file

Made by `node eval/late/run.mjs report eval/late/rows.jsonl` from the recorded runs. The bars were fixed in PREREG.md before the first keyed call. Nothing here was re-run.

Model muse-spark-1.3; engine torpor 2.3.1; harness commit e7107d8fd9a6d55838aeeef58f9a8b1292c85637; BASE binary sha256 fa20614f015c56ee, HEAD binary sha256 7a244749bf1fcea7; upstream api; 2026-10-29T06:25:40.290Z to 2026-10-29T06:39:01.938Z.

## Pass lines

| bar | result | met |
|---|---|---|
| HEAD saved at least 14 of 20 on the 4 late inputs | 20 of 20 | yes |
| HEAD saved at least BASE saved plus 8 on the 4 late inputs | HEAD 20, BASE 4 (of 20 each) | yes |
| on the 2 controls HEAD saved at least BASE saved minus 2 (of 10) | HEAD 10 of 10, BASE 10 of 10 | yes |
| HEAD junk on A2 at most 1 of 5 | 0 of 5 | yes |

No run was ended by a timeout.

## Tallies

`saved` = the verdict is check and the engine saved a check. `note` = saved and the reason is not empty (the case that puts a note in the AI's context). `junk` = the reason names the request (key words of PREREG.md).

| input | kind | arm | runs | verdict check | saved | note | junk |
|---|---|---|---|---|---|---|---|
| A2 | late | BASE | 5 | 0 | 0 | 0 | 0 |
| A2 | late | HEAD | 5 | 5 | 5 | 0 | 0 |
| N1 | late | BASE | 5 | 4 | 4 | 0 | 0 |
| N1 | late | HEAD | 5 | 5 | 5 | 0 | 0 |
| N2 | late | BASE | 5 | 0 | 0 | 0 | 0 |
| N2 | late | HEAD | 5 | 5 | 5 | 0 | 0 |
| N3 | late | BASE | 5 | 0 | 0 | 0 | 0 |
| N3 | late | HEAD | 5 | 5 | 5 | 0 | 0 |
| A1 | early | BASE | 5 | 5 | 5 | 0 | 0 |
| A1 | early | HEAD | 5 | 5 | 5 | 0 | 0 |
| A3 | early | BASE | 5 | 5 | 5 | 0 | 0 |
| A3 | early | HEAD | 5 | 5 | 5 | 0 | 0 |
| late total | late | BASE | 20 | 4 | 4 | 0 | 0 |
| late total | late | HEAD | 20 | 20 | 20 | 0 | 0 |
| controls total | early | BASE | 10 | 10 | 10 | 0 | 0 |
| controls total | early | HEAD | 10 | 10 | 10 | 0 | 0 |

## Every run

Sorted by input, arm and repetition. A reason is scored from the last writer answer that parsed; `none` = no usable answer (scores 0 for junk and saved).

| input | arm | rep | verdict | saved | junk | engine said | seconds | reason |
|---|---|---|---|---|---|---|---|---|
| A2 | BASE | 1 | cannot_check | 0 | 0 | advisory | 4.5 | No health route path appears in the provided server.js excerpt. |
| A2 | BASE | 2 | cannot_check | 0 | 0 | advisory | 3 | The health endpoint path is not visible in the provided code so it cannot be requested. |
| A2 | BASE | 3 | cannot_check | 0 | 0 | advisory | 3.8 | No health route appears in the visible server code so its path cannot be known. |
| A2 | BASE | 4 | cannot_check | 0 | 0 | advisory | 3.8 | The provided server.js excerpt shows no health route to request. |
| A2 | BASE | 5 | cannot_check | 0 | 0 | advisory | 2.9 | No health route path is visible in the provided server code. |
| A2 | HEAD | 1 | check | 1 | 0 | proven | 26.2 | (empty) |
| A2 | HEAD | 2 | check | 1 | 0 | proven | 27.2 | (empty) |
| A2 | HEAD | 3 | check | 1 | 0 | proven | 26.5 | (empty) |
| A2 | HEAD | 4 | check | 1 | 0 | proven | 26.4 | (empty) |
| A2 | HEAD | 5 | check | 1 | 0 | proven | 27.4 | (empty) |
| N1 | BASE | 1 | check | 1 | 0 | advisory | 12.3 | (empty) |
| N1 | BASE | 2 | check | 1 | 0 | advisory | 12.4 | (empty) |
| N1 | BASE | 3 | check | 1 | 0 | advisory | 12 | (empty) |
| N1 | BASE | 4 | cannot_check | 0 | 0 | advisory | 7.1 | The visible code does not show the greeting route path or response. |
| N1 | BASE | 5 | check | 1 | 0 | advisory | 12.9 | (empty) |
| N1 | HEAD | 1 | check | 1 | 0 | proven | 26.7 | (empty) |
| N1 | HEAD | 2 | check | 1 | 0 | proven | 25.5 | (empty) |
| N1 | HEAD | 3 | check | 1 | 0 | proven | 27.3 | (empty) |
| N1 | HEAD | 4 | check | 1 | 0 | proven | 26.8 | (empty) |
| N1 | HEAD | 5 | check | 1 | 0 | proven | 26.3 | (empty) |
| N2 | BASE | 1 | cannot_check | 0 | 0 | advisory | 4 | No shout route appears in the visible server code so there is no path to request. |
| N2 | BASE | 2 | cannot_check | 0 | 0 | advisory | 3.4 | No shout route appears in the provided server code to test. |
| N2 | BASE | 3 | cannot_check | 0 | 0 | advisory | 4.7 | No shout route appears in the visible server code so there is no path to check. |
| N2 | BASE | 4 | cannot_check | 0 | 0 | advisory | 2.8 | The repo shows no shout route to request. |
| N2 | BASE | 5 | cannot_check | 0 | 0 | advisory | 2.3 | No shout route appears in the provided server code. |
| N2 | HEAD | 1 | check | 1 | 0 | proven | 27.6 | (empty) |
| N2 | HEAD | 2 | check | 1 | 0 | proven | 27.5 | (empty) |
| N2 | HEAD | 3 | check | 1 | 0 | proven | 27.1 | (empty) |
| N2 | HEAD | 4 | check | 1 | 0 | proven | 26.5 | (empty) |
| N2 | HEAD | 5 | check | 1 | 0 | proven | 26.7 | (empty) |
| N3 | BASE | 1 | cannot_check | 0 | 0 | advisory | 3.1 | No sum route appears in the provided server code so it cannot be tested. |
| N3 | BASE | 2 | cannot_check | 0 | 0 | advisory | 4.1 | No sum route appears in the shown server code so its path cannot be known. |
| N3 | BASE | 3 | cannot_check | 0 | 0 | advisory | 3.5 | No sum route appears in the visible server code so its path and response cannot be verified. |
| N3 | BASE | 4 | cannot_check | 0 | 0 | advisory | 4.1 | No sum route appears in the visible server code so no path can be called to verify it. |
| N3 | BASE | 5 | cannot_check | 0 | 0 | advisory | 3.8 | No sum route appears in the shown server code so its path cannot be known. |
| N3 | HEAD | 1 | check | 1 | 0 | proven | 11.1 | (empty) |
| N3 | HEAD | 2 | check | 1 | 0 | proven | 10.9 | (empty) |
| N3 | HEAD | 3 | check | 1 | 0 | proven | 11.3 | (empty) |
| N3 | HEAD | 4 | check | 1 | 0 | proven | 10.2 | (empty) |
| N3 | HEAD | 5 | check | 1 | 0 | proven | 10.7 | (empty) |
| A1 | BASE | 1 | check | 1 | 0 | advisory | 10.5 | (empty) |
| A1 | BASE | 2 | check | 1 | 0 | advisory | 12.4 | (empty) |
| A1 | BASE | 3 | check | 1 | 0 | advisory | 11.1 | (empty) |
| A1 | BASE | 4 | check | 1 | 0 | advisory | 10.7 | (empty) |
| A1 | BASE | 5 | check | 1 | 0 | advisory | 9.9 | (empty) |
| A1 | HEAD | 1 | check | 1 | 0 | advisory | 9.1 | (empty) |
| A1 | HEAD | 2 | check | 1 | 0 | advisory | 12.4 | (empty) |
| A1 | HEAD | 3 | check | 1 | 0 | advisory | 12.2 | (empty) |
| A1 | HEAD | 4 | check | 1 | 0 | advisory | 9.1 | (empty) |
| A1 | HEAD | 5 | check | 1 | 0 | advisory | 10.1 | (empty) |
| A3 | BASE | 1 | check | 1 | 0 | advisory | 12.5 | (empty) |
| A3 | BASE | 2 | check | 1 | 0 | advisory | 12.3 | (empty) |
| A3 | BASE | 3 | check | 1 | 0 | advisory | 10.9 | (empty) |
| A3 | BASE | 4 | check | 1 | 0 | advisory | 12.7 | (empty) |
| A3 | BASE | 5 | check | 1 | 0 | advisory | 13.4 | (empty) |
| A3 | HEAD | 1 | check | 1 | 0 | advisory | 12.9 | (empty) |
| A3 | HEAD | 2 | check | 1 | 0 | advisory | 10.8 | (empty) |
| A3 | HEAD | 3 | check | 1 | 0 | advisory | 12.3 | (empty) |
| A3 | HEAD | 4 | check | 1 | 0 | advisory | 12.4 | (empty) |
| A3 | HEAD | 5 | check | 1 | 0 | proven | 13.3 | (empty) |

Calls: 0.4593 USD lo and 0.8174 USD hi over 60 writer calls in the runs above (exact sums; the spend ledger adds its rounded rows and is the record of spend; cap 3.00 hi).

## Notes (written after the runs, from the recorded runs; not part of the pre-registration)

- Pre-registration: `eval/late/PREREG.md` as of commit e7107d8fd9a6d55838aeeef58f9a8b1292c85637, pushed to the branch `writer-sees-late-code` at 2026-10-29T06:17:54Z (the CI run for that commit was created at the same second and finished green). The first draft, f723ff7, was never pushed alone; e7107d8 only spelled out the run-order formula and added two sentences to the section "Before this file was pushed". The harness files used for the runs are the pushed ones (the guard checked it).
- The sitting: one sitting, 06:25:40Z to 06:39:01Z, no stop, no resume, no timeout, 60 rows. There was one writer call per run and no second attempt, so 60 calls. Spend: $0.4593 lo and $0.8174 hi (exact sums of the 60 calls); the spend ledger (`scratch/torpor-s3/spend-S3x.md`, outside this repo) totals $0.4593 lo and $0.8178 hi against the $3.00 cap.
- The gap was reproduced: BASE saved 4 of 20 on the four late inputs (the rule "gap not reproduced" needs 14 or more).
- What the saved checks asked for (read from the raw run folders, which are deleted afterwards; this is not a scored bar). HEAD, all 20 late runs ended `proven` (the engine saw the check fail on the version before turn 1 and pass after) and each requests the route the claim is about: A2 `GET /api/health` 5 of 5, N1 `GET /api/greet` (with a name or without) 5 of 5, N2 `POST /api/shout` 5 of 5, N3 `GET /api/sum?a=2&b=3` 5 of 5. BASE: A2, N2 and N3 saved nothing (`cannot_check`, "no ... route appears in the visible code"). The 4 BASE saves on N1 were all `advisory` ("saved as a guard only") and all four are `POST /api/login` then `GET /api/me`, a check of the login and `me` routes that sit in the start of the file; none requests the greeting route. By the route asked for, BASE checked the code the claim is about in 0 of 20 late runs; the scored `saved` of 4 counts those guards.
- Controls: A1 and A3 saved 5 of 5 in both arms, each time a login-then-`me` guard (a few start with `GET /`, and one A3 BASE check is the login only; the excerpt is byte-identical in the two arms for these inputs, see PREREG.md). Every saved check came with an empty reason (note 0 in the table), and junk was 0 of 60 runs.
- Time: the writer call took 3 to 7 seconds in every arm. A run that ended `proven` took 26 to 27 seconds for A2, N1 and N2 and 11 seconds for N3; the rest of the run is local work (the engine proving the check). Why A2, N1 and N2 take longer than N3 was not investigated. For comparison the Stop hook waits at most 12 seconds for a save (stage S3t).
- Not measured here: the real Muse flow with the real first question (it is canned, see PREREG.md), a live AI session, files over 64,000 bytes (the writer never reads them), and a claim whose words share no word with its code.
