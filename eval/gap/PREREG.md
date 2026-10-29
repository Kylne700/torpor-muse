# S3g pre-registration: the check-writer names only what the developer said works

Written before the first keyed call. After the first keyed call this file never changes; anything learned later goes in RESULT.md. The time this file was pushed to the branch `gap-claims-only` is the pre-registration time (git and the GitHub push event); the stage report and RESULT.md quote it. `node eval/gap/run.mjs run` refuses to start a keyed run unless this file and `eval/gap/run.mjs` equal the pushed ones and the two binaries below have the hashes named here.

## The bug and the change

In the e2e proof L4c (e2e/PROOF.md, E2) the AI got a 328-character note on a clean turn. The note said the check saved for "The sign-in button works great. Add a 'Forgot password?' link under it ..." does not cover the link, "a new addition, not a claimed working feature". The link is a request, not a claim. The writer prompt line `src/suggest/prompt.rs:30` told the model to leave `reason` empty "when the steps prove everything the developer said", so a request in the same message counted as something unproven. The `reason` becomes the note.

One line of `src/suggest/prompt.rs` changes (commit 3678126b277631f9213dbe6e891dd0a481106687, parent 490cf4c5bf0c32a16dae108321ebdbf2be9c6ebe). The line was (BASE):

```
- reason: empty when the steps prove everything the developer said. Otherwise one sentence naming the part they cannot test (a restart of the server, a request header such as a bearer token, replaying an old cookie, waiting for time to pass).
```

and is now (HEAD):

```
- reason: empty when the steps prove everything the developer said works (or is fixed). Otherwise one sentence naming the part of that statement they cannot test (a restart of the server, a request header such as a bearer token, replaying an old cookie, waiting for time to pass). Never name a request, a plan or a question from the same message in reason: none of them is something that works.
```

The new sentence was written knowing the failure on input A3. The words that are new in HEAD use none of the scoring key words below. The examples of untestable parts are unchanged and are in both arms (they contain "restart" and "time", two of the kept key words).

## What is run

- Arms. BASE: the `torpor-muse` release adapter built with `cargo build --release --locked` from main 490cf4c5bf0c32a16dae108321ebdbf2be9c6ebe. HEAD: the same build from 3678126b277631f9213dbe6e891dd0a481106687. They differ in that one line.
- Binary hashes (checked by the harness before the first call):

```
base_sha256 = d1b8d63b65c65ed02c05a5c56bde1e376ca17bc57bced096e4c59c9e6996e829
head_sha256 = b846b54e8704aa41838a8bbf6bc50a824faa550beb9e91811448080b370d07ce
```

- Engine, the same for both arms: `scratch/torpor-engine/bin/torpor` (path relative to the project root), `torpor version` prints `torpor 2.3.1`, sha256 0048fc3b7ee63f3d77c9f3cb63941176b30ddd427f7f6dd8dae929241c7f2e3f.
- Model and settings: the adapter's own request, model `muse-spark-1.3` (the standard-priced id), reasoning effort low, strict JSON schema, up to two attempts when the engine turns a check down. Both arms use the same adapter code; only the prompt differs. The adapter talks to a loopback stand-in (`eval/gap/run.mjs`) that adds the real key and sends the request on to `https://api.meta.ai/v1/responses`.
- The first question is canned. The product asks a cheap first question (`claim` or `not_a_claim`) before the writer. Here the stand-in answers it with `{"verdict":"claim","reason":""}` at no cost, so that every run reaches the writer and the writer's prompt is what is measured (the first question was measured by itself in eval/first-pass). The writer never sees the first answer.
- The path is the product's hook path: `torpor-muse suggest -` with `TORPOR_MUSE_FROM_HOOK=s1` and the developer's words in `TORPOR_MUSE_WORDS` (not in argv), stdin closed, a fresh state folder per run. That is how the Stop hook starts the writer.
- History. Every run is a fresh copy of `demo/app` (git-initialised) with a two-turn history made by the real engine's hooks (`torpor hook <Event>` with the payload on stdin, session `s1`): SessionStart (`startup`), UserPromptSubmit `turn one work`, the turn-1 edit below, Stop, UserPromptSubmit with the claim words (which starts turn 2). The writer runs on that state.
- Order. Runs are numbered p = 0 to 49. Run p uses input INPUTS[p mod 5] (A1, A2, A3, C1, C2 in turn) and the arm BASE when p is even, HEAD when p is odd. BASE and HEAD alternate strictly, and in every 10 consecutive runs each input meets each arm once, so every input is run 5 times with each arm: 50 runs, in one sitting.

## Inputs and app states

Each input is a fresh copy of `demo/app/server.js` (one file) with the edits below. A "text replacement" must match exactly once.

| id | words (verbatim) | app state | turn-1 edit |
|---|---|---|---|
| A1 | Login works now, thanks. Please add a 'Show password' checkbox under the password field. | as is | HEADING |
| A2 | The health route works. Add a short comment above checkLogin that says what it returns. | as is | HEALTH |
| A3 | The sign-in button works great. Add a 'Forgot password?' link under it that points to #. | as is | HEADING |
| C1 | Login works now, and it still works after the server restarts. Please add a 'Show password' checkbox under the password field. | as is | HEADING |
| C2 | The sign-in button works great, and the session ends after 30 minutes. Add a 'Forgot password?' link under it that points to #. | SESSION30 from the start | HEADING |

A1 to A3 are the three `// claim` lines of `CLEAN_PROMPTS` in e2e/proof/scripts.mjs (the selftest compares them). A1 to A3 contain a claim that the steps can prove fully plus a request: the right `reason` is empty. C1 and C2 contain a claim with one part that HTTP steps cannot prove (a restart; waiting 30 minutes) plus a request: the right `reason` names that part and not the request.

Edits (text replacements on `server.js`; the first line of each pair is replaced by the second):

HEADING (turn 1 of A1, A3, C1, C2: an unrelated edit, so that turn 1 has a diff):

```
<h1>Login demo</h1>
<h1>Welcome back</h1>
```

HEALTH (turn 1 of A2: the route the claim "The health route works" is about):

```
  res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
```

is replaced by

```
  if (req.method === "GET" && path === "/api/health") {
    sendJson(res, 200, { ok: true });
    return;
  }

  res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
```

SESSION30 (C2, applied before the first commit, so the claim "the session ends after 30 minutes" is true in the app): three replacements.

```
      sessions.set(token, true);
```

becomes

```
      sessions.set(token, Date.now() + 30 * 60 * 1000);
```

```
SameSite=Lax; Path=/",
```

becomes

```
SameSite=Lax; Path=/; Max-Age=1800",
```

```
sessions.has(token)
```

becomes

```
sessions.get(token) > Date.now()
```

The selftest starts each patched app and checks it: `/api/health` answers 404 before HEALTH and 200 `{"ok":true}` after; with SESSION30 the login cookie has `Max-Age=1800` and `/api/me` accepts it.

## Scoring (a script, `report` in eval/gap/run.mjs; case-insensitive substring match)

- The scored answer is the last writer answer that parsed (HTTP 200, a JSON object with a string `verdict` and a string `reason`), whatever its verdict. Its `reason` is the text that is scored. If no writer answer parsed, the run's verdict is `none` and it scores 0 for junk, kept and saved.
- junk = 1 when the `reason` contains one of the junk key words of the input (it names the request). Defined for all 25 runs of an arm.
- kept = 1 when the `reason` contains one of the kept key words (it names the real gap). Defined for C1 and C2 only (10 runs of an arm).
- saved = 1 when the verdict is `check` and the engine saved a check (`saved: true` in the run's result.json). A check saved as a guard only ("no earlier version fails it") counts as saved.
- note = 1 when the check is saved and the `reason` is not empty (the case that puts a note in the AI's context). It is reported in the tallies and is not a pass line.

Key words, one line per input (the selftest wants these lines here word for word):

```
A1: junk "show password" or "checkbox".
A2: junk "comment".
A3: junk "forgot" or "link".
C1: junk "show password" or "checkbox"; kept "restart".
C2: junk "forgot" or "link"; kept "30 minutes" or "expire" or "time" or "ends".
```

Substring matching is loose on purpose and fixed now ("link" also matches "unlinked", "time" also matches "timeout"); no reason is re-scored by hand.

## Pass lines (HEAD against BASE, over all 50 runs)

1. HEAD junk is at most 1 of 25.
2. HEAD kept is at least BASE kept minus 1 (of 10).
3. HEAD saved is at least BASE saved minus 2 (of 25).

If BASE junk is 0 of 25, RESULT.md says "bug not reproduced" next to the pass lines (the pass lines are still computed and shown).

## Money and stopping

- Keyed calls only while `/tmp/claude-1000/meta-billing-ok` exists; the stand-in checks it before every forwarded call. Every keyed run holds `flock /tmp/claude-1000/meta-keyed.lock`. The key reaches the harness only through the environment, read from the key file inside the same command; no child process sees it.
- Cap $3.00 hi for the stage, enforced by the stand-in before every forwarded call (the worst case of the call is added to the ledger's hi total). Near the cap, or if the flag is gone: stop, report, never go past it. A stopped sitting leaves the missing runs missing; RESULT.md then says INCOMPLETE.
- An error of the provider or the network is not a result. Any answer from the API other than HTTP 200, or no answer, stops the sitting at once, after the failed call's ledger row (counted at its worst case when no usage came back). The run in flight is dropped, not recorded, and runs again from scratch if the sitting is resumed. After a transient error (HTTP 429, 5xx, a timeout) the stage resumes, at most 3 times in all; after a refusal of the key or the request (HTTP 400, 401, 402, 403, 404) it does not resume and reports to the lead. An HTTP 200 answer is always a result, even one the adapter cannot use (it is scored as above). RESULT.md lists every stop.
- Ledger: `scratch/torpor-s3/spend-S3g.md` (relative to the project root), one row per forwarded response, written by the stand-in from the recorded usage, columns as spend-L4-proof.md, with a total line against $3.00.

## After the first keyed call

- Nothing is re-run because a result is bad. A run that did not finish (a crash, a kill, a stop, an error of the provider or the network) is the only reason to start the same command again; it skips the runs already recorded.
- Results go to eval/gap/RESULT.md (made by `report` from eval/gap/rows.jsonl) whatever they show. Anything learned later is written there as a note, never here.

## Known limits

- 5 runs per input and arm. A pass shows no gross harm and no large junk; it is not a proof of either.
- Key-word scoring is a proxy: it does not read meaning. RESULT.md prints every reason so a reader can judge.
- The first question is canned (above); a real `claim` verdict on these inputs is expected, not measured here.
- The history is made by the real engine's hooks, not by a live AI session. For A1, A3, C1, C2 the engine has no earlier version that fails the check, so those checks are saved as guards only; for A2 the check can be proven (it fails before turn 1 and passes after).
- The BASE failure rate is unknown before the runs; that is why "bug not reproduced" is an allowed result.
- Same-day provider variance and the model's own randomness are not controlled.

## Before this file was pushed

No keyed call had been made. The harness passed its selftest (which also checks this file's key-word lines and edit texts, and the stand-in against a local fake upstream: the canned first question, the ledger row, the flag, the cap, and the stop on an HTTP 401 and on a refused connection).

It was rehearsed against the local mock server (`mock/server.mjs`) with the real adapters and the real engine:

- Twice with 50 runs: once with key words in the mock's reason (junk 25 of 25 in both arms, so pass line 1 showed NO, as it must) and once with an empty reason (junk 0, "bug not reproduced"). These two used an earlier `run.mjs` that differs from the pushed one in the stop on an error of the provider or the network, in two wording changes and in one test string.
- Once with the pushed `run.mjs`, as a stop-and-resume test: a loopback upstream answering HTTP 401 stopped the sitting at the first call with exit code 3 and no run recorded, and the resume against the mock then ran all 50 runs (one stop and one resume listed in the report header).

The call to the real API (`fetch` with the key as a bearer token, nothing else of the adapter's request) is the same line as in eval/first-pass/run.mjs, whose 65 real calls all answered HTTP 200. For this stage it has not run before the first keyed call.
