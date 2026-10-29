# S3x pre-registration: the check-writer sees the code a claim is about, also far into a file

Written before the first keyed call. After the first keyed call this file never changes; anything learned later goes in RESULT.md. The time this file was pushed to the branch `writer-sees-late-code` is the pre-registration time (git and the GitHub push event); the stage report and RESULT.md quote it. `node eval/late/run.mjs run` refuses to start a keyed run unless this file, `eval/late/run.mjs` and the two harness files it imports (`eval/gap/run.mjs`, `eval/first-pass/run.mjs`) equal the pushed ones and the two binaries below have the hashes named here.

## The gap and the change

The gap. `src/suggest/repo/excerpt.rs` kept only the first 6,000 bytes of each file (`FILE_CAP`) and filled up to 14,000 bytes in all (`TOTAL_CAP`) with file starts. A claim about code later in a file reached the model without that code. In the S3g A/B (eval/gap/rows.jsonl) input A2, whose health route sits past the cut, got the verdict `cannot_check` in 10 of 10 runs (5 BASE, 5 HEAD of that A/B), with reasons such as "No health route path appears in the visible server code to test."; input A1, whose code is at the start of the file, saved 5 of 5 in both arms. The writer's request holds two data blocks only, `repo_files` and `developer_words`: it sees no diff.

The change (commit 4c7a5f8f109878f46461fd11ff1b9b5533058e82 on top of the test commit 30303e235b9ea64871cf116ead0dcebc4558b501, parent main 570bc26bcbcb193be066fab9a7a97e52f34c35ea). The excerpt keeps the file starts exactly as before, in the same order (the first 6,000 bytes of a file, then `[cut]`). After that it adds "windows": blocks of code from the later part of a file (a file the writer may read, up to 64,000 bytes) that hold words of the developer's message. A hit line grows into the block it sits in (at most 40 lines or 1,800 bytes, with the comment above it), the best blocks are appended after the file's start, at most 4,000 bytes per file, and only from room that is still free under the 14,000 bytes in all. A file whose start was cut by the total cap gets no window. The whole text is redacted before any cut, words without a hit give exactly the old excerpt, and there is no new option or setting and no extra model call. `src/suggest/prompt.rs` is not touched.

Offline result of the change, no model (`cargo test --test writer_late_code`: 18 claims about code in six made-up apps of 7 to 60 KB, `tests/fixtures/late`; a claim counts as seen when all the lines that implement it are in the excerpt in one piece):

| | late claims seen (12) | early claims seen (6) | longest excerpt |
|---|---|---|---|
| BASE 570bc26 | 0 of 12 | 6 of 6 | 12,035 bytes |
| HEAD 4c7a5f8 | 12 of 12 | 6 of 6 | 13,964 bytes (cap 14,000) |

This A/B asks whether that shows in what the model does with the real writer prompt.

## What is run

- Arms. BASE: the `torpor-muse` release adapter built with `cargo build --release --locked` from main 570bc26bcbcb193be066fab9a7a97e52f34c35ea (a `git archive` of it, built in an empty folder). HEAD: the same build from 4c7a5f8f109878f46461fd11ff1b9b5533058e82, the last commit of this branch that changes `src/`; at the time of writing the commits after it add only `eval/late/**`. They differ in the excerpt code only; the prompt is the same (S3g's).
- Binary hashes (checked by the harness before the first call):

```
base_sha256 = fa20614f015c56eea51f3d27daf11f5eebe5528a50400bd5beb421194d87533f
head_sha256 = 7a244749bf1fcea7905fc1cb87c35d56ffc34cd4ad920d962aa2c07ac946b622
```

- Engine, the same for both arms: `scratch/torpor-engine/bin/torpor` (path relative to the project root), `torpor version` prints `torpor 2.3.1`, sha256 0048fc3b7ee63f3d77c9f3cb63941176b30ddd427f7f6dd8dae929241c7f2e3f.
- Model and settings: the adapter's own request, model `muse-spark-1.3` (the standard-priced id), reasoning effort low, strict JSON schema, up to two attempts when the engine turns a check down. Both arms use the same adapter code except the excerpt. The adapter talks to a loopback stand-in (`eval/gap/run.mjs`, as in S3g) that adds the real key and sends the request on to `https://api.meta.ai/v1/responses`.
- The first question is canned. The product asks a cheap first question (`claim` or `not_a_claim`) before the writer. Here the stand-in answers it with `{"verdict":"claim","reason":""}` at no cost, so that every run reaches the writer and the writer's view of the code is what is measured. The writer never sees the first answer.
- The path is the product's hook path: `torpor-muse suggest -` with `TORPOR_MUSE_FROM_HOOK=s1` and the developer's words in `TORPOR_MUSE_WORDS` (not in argv), stdin closed, a fresh state folder per run, a limit of 300 seconds per run. That is how the Stop hook starts the writer.
- History. Every run is a fresh copy of `demo/app` (git-initialised, with the `.synthetic` marker) with a two-turn history made by the real engine's hooks (`torpor hook <Event>` with the payload on stdin, session `s1`): SessionStart (`startup`), UserPromptSubmit `turn one work`, the turn-1 edit below, Stop, UserPromptSubmit with the claim words (which starts turn 2). The writer runs on that state.
- Order. Runs are numbered p = 0 to 59. Run p uses the arm BASE when p is even and HEAD when p is odd, and the input INPUTS[(p + (floor(p / 6) mod 2)) mod 6] (the list is A2, N1, N2, N3, A1, A3; runs 0 to 5, 12 to 17, 24 to 29, ... take it in that order, runs 6 to 11, 18 to 23, 30 to 35, ... take it shifted by one place, N1 first). BASE and HEAD alternate strictly, and in every 12 consecutive runs each input meets each arm once, so every input is run 5 times with each arm: 60 runs, in one sitting. (A plain p mod 6 would give each input one arm only, because 6 is even.)

## Inputs and app states

Each input is a fresh copy of `demo/app/server.js` (one file, 6,775 bytes) with the turn-1 edit below. A "text replacement" must match exactly once. Late: the code the claim is about sits past byte 6,000 of `server.js`, where the BASE writer's view ends (every route below goes in front of the 404 handler, which starts at byte 6,451). Early: it sits in the first 6,000 bytes.

| id | kind | words (verbatim) | turn-1 edit |
|---|---|---|---|
| A2 | late | The health route works. Add a short comment above checkLogin that says what it returns. | HEALTH |
| N1 | late | The greeting route works. Add a 'Show password' checkbox under the password field. | GREET |
| N2 | late | The shout route works. Add a 'Forgot password?' link under the sign-in button that points to #. | SHOUT |
| N3 | late | The sum route works. Change the page title to 'Sign in'. | SUM |
| A1 | early | Login works now, thanks. Please add a 'Show password' checkbox under the password field. | HEADING |
| A3 | early | The sign-in button works great. Add a 'Forgot password?' link under it that points to #. | HEADING |

A1, A2 and A3 are the inputs of the same name in eval/gap/PREREG.md (same words, same edits; the selftest compares them). N1 to N3 are new: each is a claim about a route that the turn-1 edit adds, so the claim is true in the app, plus a request in the same message that is not a claim (as in S3g). None of the three is one of the 18 claims of the offline table. The right `reason` for all six is empty, and the right verdict is `check`: the steps can prove the route.

Edits (text replacements on `server.js`; the first block of a pair is replaced by the second):

HEADING (turn 1 of A1 and A3: an unrelated edit, so that turn 1 has a diff):

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

GREET (turn 1 of N1: the same 404 line is replaced by):

```
  if (req.method === "GET" && path === "/api/greet") {
    const name = new URL(rawUrl, "http://localhost").searchParams.get("name");
    sendJson(res, 200, { greeting: "Hello, " + (name ? name : "friend") + "!" });
    return;
  }

  res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
```

SHOUT (turn 1 of N2: the same 404 line is replaced by):

```
  if (req.method === "POST" && path === "/api/shout") {
    const chunks = [];
    req.on("data", (chunk) => chunks.push(chunk));
    req.on("end", () => {
      let parsed;
      try {
        parsed = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      } catch (err) {
        sendJson(res, 400, { ok: false });
        return;
      }
      if (parsed === null || typeof parsed.text !== "string" || parsed.text === "") {
        sendJson(res, 400, { ok: false });
        return;
      }
      sendJson(res, 200, { ok: true, text: parsed.text.toUpperCase() + "!" });
    });
    return;
  }

  res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
```

SUM (turn 1 of N3: the same 404 line is replaced by):

```
  if (req.method === "GET" && path === "/api/sum") {
    const params = new URL(rawUrl, "http://localhost").searchParams;
    const a = Number(params.get("a"));
    const b = Number(params.get("b"));
    if (params.get("a") === null || params.get("b") === null || !Number.isFinite(a) || !Number.isFinite(b)) {
      sendJson(res, 400, { ok: false });
      return;
    }
    sendJson(res, 200, { sum: a + b });
    return;
  }

  res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
```

The selftest starts each patched app and checks it: before its patch each route answers 404; `/api/health` answers 200 `{"ok":true}`; `/api/greet?name=Dana` answers `{"greeting":"Hello, Dana!"}` and `/api/greet` without a name `Hello, friend!`; POST `/api/shout` with `{"text":"hi"}` answers 200 `{"ok":true,"text":"HI!"}` and 400 for an empty or invalid body; `/api/sum?a=2&b=3.5` answers `{"sum":5.5}` and 400 when a number is missing or not a number.

## Scoring (a script, `report` in eval/late/run.mjs, with the scorer of eval/gap/run.mjs; case-insensitive substring match)

- The scored answer is the last writer answer that parsed (HTTP 200, a JSON object with a string `verdict` and a string `reason`), whatever its verdict. Its `reason` is the text that is scored. If no writer answer parsed, the run's verdict is `none` and it scores 0 for junk and saved.
- saved = 1 when the verdict is `check` and the engine saved a check (`saved: true` in the run's result.json). A check saved as a guard only counts as saved.
- junk = 1 when the `reason` contains one of the junk key words of the input (it names the request, not a part of the claim). Only the A2 runs of HEAD are a pass line; it is counted for all runs.
- note = 1 when the check is saved and the `reason` is not empty (the case that puts a note in the AI's context). It is reported in the tallies and is not a pass line.
- A run ended by the 300 second limit is recorded with a flag, counts as not saved, and is listed in RESULT.md.

Key words, one line per input (the selftest wants these lines here word for word):

```
A2: junk "comment".
N1: junk "show password" or "checkbox".
N2: junk "forgot" or "link".
N3: junk "title".
A1: junk "show password" or "checkbox".
A3: junk "forgot" or "link".
```

Substring matching is loose on purpose and fixed now ("link" also matches "unlinked"); no reason is re-scored by hand.

## Pass lines (HEAD against BASE, over all 60 runs; the bars are the script's `passLines`, which the selftest compares with this list)

1. HEAD saved at least 14 of 20 on the 4 late inputs
2. HEAD saved at least BASE saved plus 8 on the 4 late inputs
3. on the 2 controls HEAD saved at least BASE saved minus 2 (of 10)
4. HEAD junk on A2 at most 1 of 5

The late inputs are A2, N1, N2 and N3 (5 runs each per arm, 20 per arm); the controls are A1 and A3 (10 runs per arm). If BASE saved 14 or more of 20 on the late inputs, RESULT.md says "gap not reproduced" next to the pass lines (the pass lines are still computed and shown).

## Money and stopping

- Keyed calls only while `/tmp/claude-1000/meta-billing-ok` exists; the stand-in checks it before every forwarded call. Every keyed run holds `flock /tmp/claude-1000/meta-keyed.lock`. The key reaches the harness only through the environment, read from the key file inside the same command; no child process sees it.
- Cap $3.00 hi for the stage, enforced by the stand-in before every forwarded call (the worst case of the call is added to the ledger's hi total). Near the cap, or if the flag is gone: stop, report, never go past it. A stopped sitting leaves the missing runs missing; RESULT.md then says INCOMPLETE. Expected: about $0.5 lo and $0.9 hi for 60 runs (S3g: 50 runs cost $0.42 lo and $0.74 hi).
- An error of the provider or the network is not a result. Any answer from the API other than HTTP 200, or no answer, stops the sitting at once, after the failed call's ledger row (counted at its worst case when no usage came back). The run in flight is dropped, not recorded, and runs again from scratch if the sitting is resumed. After a transient error (HTTP 429, 5xx, a timeout) the stage resumes, at most 3 times in all; after a refusal of the key or the request (HTTP 400, 401, 402, 403, 404) it does not resume and reports to the lead. An HTTP 200 answer is always a result, even one the adapter cannot use (it is scored as above). RESULT.md lists every stop.
- Ledger: `scratch/torpor-s3/spend-S3x.md` (relative to the project root), one row per forwarded response, written by the stand-in from the recorded usage, columns as spend-L4-proof.md, with a total line against $3.00.

## After the first keyed call

- Nothing is re-run because a result is bad. A run that did not finish (a crash, a kill, a stop, an error of the provider or the network) is the only reason to start the same command again; it skips the runs already recorded.
- Results go to eval/late/RESULT.md (made by `report` from eval/late/rows.jsonl) whatever they show. Anything learned later is written there as a note, never here.

## Known limits

- 5 runs per input and arm. A pass shows no gross harm and a large gain on these routes; it is not a proof of either.
- Key-word scoring is a proxy: it does not read meaning. RESULT.md prints every reason so a reader can judge.
- The app is one file of 6.8 KB, so the 14,000 bytes in all are never short here. A project whose file starts already fill the 14,000 bytes leaves no room for windows, and a claim about its later code gets none; the offline table (apps up to 60 KB) shows the effect of the change without a model, this A/B only on one small app.
- The late claims are about routes whose names are words of the developer's message ("health", "greeting", "shout", "sum"). A claim that names no word of its code is not helped.
- The first question is canned (above); a real `claim` verdict on these inputs is expected, not measured here.
- The history is made by the real engine's hooks, not by a live AI session. For A1 and A3 the engine has no earlier version that fails the check, so those checks are saved as guards only; for A2, N1, N2 and N3 the route is absent before turn 1 and present after, as for A2 in eval/gap/PREREG.md.
- The BASE rate on the late inputs is expected to be low but is not known before the runs; "gap not reproduced" is an allowed result.
- Same-day provider variance and the model's own randomness are not controlled.

## Before this file was pushed

No keyed call had been made. The checks of the change were green (fmt, clippy with `-D warnings`, 499 tests passed and 0 failed at HEAD against 482 and 0 at BASE, the e2e script "21 of 21", `scripts/test/run.sh` 19 passed, `scripts/accept.sh` "accept: OK", the privacy counts all 0).

The harness passed its selftest (which also checks this file's inputs, key-word lines, edit texts, hash lines and pass lines, the four patched apps, the order, the scorer, the pass-line edges, the report, and the ledger header and total against a local fake upstream).

Just before the push the HEAD binary was rebuilt from the committed source with `cargo build --release --locked`: cargo compiled nothing and the file had the `head_sha256` above. The two arms differ in the excerpt only: in `src/suggest/plan.rs` the developer's words go into the request in the same redacted, capped form in both arms (HEAD builds them once, before the excerpt, and passes them to both).

`node eval/late/run.mjs seen` ran the real engine's history for every input and both binaries against the local mock server (`mock/server.mjs`) and read the `repo_files` block of the writer's request. "Route-only lines" are the lines of the route that the rest of `server.js` does not have:

```
input kind  arm  excerpt bytes  the route block in it
A2    late  base          6043  block not whole; 0 of 2 route-only lines
A2    late  head          6460  whole block; 2 of 2 route-only lines
N1    late  base          6043  block not whole; 0 of 3 route-only lines
N1    late  head          6281  whole block; 3 of 3 route-only lines
N2    late  base          6043  block not whole; 0 of 5 route-only lines
N2    late  head          6649  whole block; 5 of 5 route-only lines
N3    late  base          6043  block not whole; 0 of 8 route-only lines
N3    late  head          6476  whole block; 8 of 8 route-only lines
A1    early base          6043  excerpt is the BASE one
A1    early head          6043  excerpt is identical to the BASE one
A3    early base          6043  excerpt is the BASE one
A3    early head          6043  excerpt is identical to the BASE one
```

The excerpt depends on the files and the words only, so it is the same in the five runs of an input.

The 60 runs were rehearsed once against the local mock server with the real adapters and the real engine (no key, no flag, a loopback upstream that answers every writer call with a health-route check): 60 rows were recorded, strictly alternating, with no timeout, A2 saved 5 of 5 in both arms and every other input 0 of 5 (the canned check fails on routes it was not written for), and the pass lines showed NO, as they must for a mock that does not look at the excerpt. The rehearsal took 9 minutes 10 seconds. It used the `run.mjs` that is pushed with this file.

The call to the real API (`fetch` with the key as a bearer token, nothing else of the adapter's request) is in `startProxy` of `eval/gap/run.mjs`, the code that made the 50 real calls of S3g. For this stage it has not run before the first keyed call.
