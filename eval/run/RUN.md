# Eval runner

Plays every episode (task x arm x rep) through real Muse Code turns, scores it with the held-out oracle, and writes one JSON row per
attempt. The tasks and oracles are described in `eval/DESIGN.md`; this file is about the runner in `eval/run/`.

    run.mjs      the command line: plan, resume, groups, spend cap, summary
    episode.mjs  one episode: workspace, arm install, six turns, injection, oracle, row
    arms.mjs     the four arms, one function each
    sandbox.mjs  one function that runs a command inside the Muse sandbox
    lib.mjs      seeded plan, prices, session-log tally, row schema, spend, held-out check
    selftest.mjs offline check (fake Muse), under 60 s: node eval/run/selftest.mjs

## Run it

    # free, no model (the ECHO provider): checks the plumbing and the hooks
    node eval/run/run.mjs --echo --tasks notes-ctrl --arms A,B,C,D --reps 1 --seed 1 --out $OUT

    # keyed: the key goes in through the environment of this one command, never a flag, a file or argv
    META_API_KEY="$(cat $KEYFILE)" node eval/run/run.mjs --tasks all --arms A,B,C,D --reps 1 --seed 1 \
        --model muse-spark-1.3-contributor --cap-usd 12 --out $OUT --run-id pilot-1 --tmp /tmp/eval-run

`--tasks all|id,id`  `--arms A,B,C,D`  `--reps N`  `--seed S`  `--model ID` (keyed; it is passed to every `muse exec`)  `--cap-usd X` (keyed)
`--out DIR`  `--run-id ID`  `--concurrency 3`  `--max-steps 40` (per turn)  `--turn-timeout-s 420`  `--episode-cap-s 900`  `--effort medium`
`--muse DIR` (the Muse install folder)  `--engine BIN`  `--adapter BIN` (default: `target/release/torpor-muse` of this checkout)
`--writer-key-file FILE` (arm D, keyed; mode 0600)  `--writer-day-cap USD` (default 0.05, per episode)  `--tmp DIR` (under /tmp)
`--keep-ep` (keep the episode folders)  `--dry-run` (print the plan). Without the muse/engine/out flags the defaults are
`scratch/torpor-g0/muse-install`, `scratch/torpor-engine/bin/torpor` and `scratch/torpor-s4a/results/<run-id>` under the nearest folder
above the repo that has a `scratch/` folder. `--tmp` shows up in the model's working-directory path: use a neutral name.

* The plan is seeded: groups in (rep, task) order, the arms inside a group in an order drawn from the seed and the group alone, recorded as
  `order_index`. The same flags give the same plan; `plan.json` in the out folder refuses a different plan or run id on resume.
* Resumable: rerun the same command. Finished episodes are skipped, rows are written atomically (`rows/<task>__<arm>__r<rep>__a<attempt>.json`).
* The pilot and the main run are never pooled: give each its own `--run-id` and its own `--out` folder (a folder holds one run id).

## Arms (one function each in `arms.mjs`; the row's `manifest` records what was installed)

* **A vanilla.** Nothing: no hooks, no instruction file, not even a settings.json.
* **B notes file.** `AGENTS.md` in the workspace root. It never mentions Torpor. Exact text:

      # Project notes

      Keep a file called NOTES.md in the project root.

      - Whenever the user tells you that something works, add a line to NOTES.md that says what works and how to check it (a command to run or a request to make).
      - Before you tell the user that a task is done, go through NOTES.md and check every entry again. If one of them no longer works, fix it first, or say so plainly.
      - Keep NOTES.md short and current: remove entries for behaviour that was removed on purpose.

* **C engine hand-wired, no adapter.** Muse's `settings.json` (in the episode HOME) holds the engine's ten documented events, each as
  `<engine> hook <Event>`, with the engine's own tool matcher on the three tool events and its documented timeouts, and nothing else.
  Muse uses the same event names, so the mapping is the identity; it lives in ONE place, `ENGINE_EVENTS` and `armCSettings` in
  `arms.mjs`. The drill's arm 3 must produce the same file: `node eval/run/arms.mjs --print-arm-c` prints it with the engine path as a
  placeholder, and its sha256 (`shape_sha256` in the manifest). Engine state: `$HOME/.torpor` of the episode.
* **D adapter + engine.** `torpor-muse init` run inside the episode HOME (engine state in `<episode>/torpor-home`), built from this checkout
  (`adapter_commit`, `adapter_sha256`, `adapter_version` in the manifest). In keyed runs with `--writer-key-file` the writer config
  (`key_file`, `day_cap_usd`, mode 0600) is written too and `manifest.writer` is `on`; otherwise it is `off`. The writer uses the adapter's
  default model (standard prices, not the contributor id). It is not verified through hooks until the A10 fix is on main.

## One episode

1. A fresh folder under `--tmp`; `make-workspace.mjs <task> ws` (start state, git repo, `.synthetic` marker); HOME, TMPDIR and TORPOR_HOME inside it.
2. Turns 1 to 6 from `task.json`: each is one `muse exec --session-id <same id> --prompt-file ... --max-model-steps 40` inside the sandbox.
   **Multi-turn mechanism (measured, ECHO and keyed):** a second `exec` with the same session id continues the conversation (a keyed nonce
   given in turn 1 was repeated in turn 2). The fidelity gap: every exec fires SessionStart (`startup` in turn 1, `resume` after), UserPromptSubmit,
   Stop and, on exit, SessionEnd, so a six-turn episode has five extra resume boundaries that one interactive session would not have.
   The SessionEnd run is not written to `session.jsonl`; hook counts cover SessionStart, UserPromptSubmit, tool events and Stop.
3. Held out: `task.json`, the oracle, the patches, results and other episodes are never inside a model sandbox (only the episode folder is
   bound read-write). The turn prompt is written into the episode just before the turn and removed after it. After every turn the episode
   folder is scanned (by name and by content hash) for held-out files; a hit makes the row invalid (`held_out_violation`, not re-run).
4. Injected tasks: after turn 3 the host applies `inject.patch` (`git apply --recount -C1 --whitespace=nowarn`), runs the oracle once and
   records `bit`: a `risk.checks` check failed. A patch that does not apply is `inject_failed` (invalid, not re-run). A patch that applied
   but did not bite stays a valid row with `bit: false`.
5. Score: wait up to 60 s for detached writers (arm D), then run `node eval/oracles/<app>.mjs --task <id> --dir <ws>` in a sandbox with no
   network. Caps: 6 turns, 15 minutes wall clock (a timeout counts as a fail: `final_pass` and `later_kept` false, the oracle still recorded).
6. Infra failures (sandbox error, Muse crash, oracle exit 2, 429 after the global backoff) make the row invalid with a reason; the episode is
   re-run up to 2 more times, every attempt keeps its row. Fatal errors (rejected key, unknown model id, spend cap) stop the run and are not
   re-run. A turn that hits `--max-model-steps` is the model's failure, not infra.

## Whole groups

A group is one (task, rep) with all its arms. One worker plays a group's arms one after the other, so the pair runs close in time, and a
group is only started when the cap leaves room for all of it (`Spend.groupFits`: spent plus one dearest finished episode per episode still to
run). A stop (cap, fatal error, first SIGINT or SIGTERM) starts no new group and lets the groups in progress finish, so the rows left behind
are whole groups. Only a second signal kills what is running; that, or a crash, can leave a half group, and the same command finishes it.
`summary.json` has `groups_planned` and `groups_complete`.

## Sandbox

`sandbox.mjs` runs everything the model touches (and the oracle, patch and init runs) behind one function with the CLI of
`scripts/sandbox/muse-sandbox.sh` (`--run-dir`, `--ro`, `--net none|meta`, `--allow`, `--chdir`). That script is used when it exists;
until then the bwrap flags of the confinement rule (v2.1) are used: read-only root, the home folder hidden, only the episode folder writable.
`EVAL_SANDBOX=v2.1|v3` forces one. **Gap under v2.1:** keyed runs share the host network, so keyed runs are limited to concurrency 1.
Model sandboxes never use a private pid namespace (an arm D writer must outlive Muse); oracle, patch and init sandboxes do.

## Spend

After every turn the runner sums `model_completed.usage` from the episode's session logs plus the writer's ledger lines, at Meta's list
prices (contributor ids: cached 0.002, input 0.10, output 0.20 USD per 1M tokens; standard: 0.15, 1.25, 4.25). `usd_lo` counts cached tokens
once at the cached price; `usd_hi` also counts them at the input price and adds reasoning tokens to output, so `usd_hi` is the bound the cap
uses. Before each turn the runner refuses to start if spent plus the dearest turn so far (once per running turn) would pass `--cap-usd`.
`spend.jsonl` in the out folder has one line per turn and per late writer charge. Rate limits: a 429 or Retry-After from Muse pauses every
worker (the pause is not counted in the 15 minutes).

## Row (schema 1; fields are added, never renamed)

`schema run_id task app kind torpor_should_help arm rep attempt seed order_index model muse_version engine_version adapter_commit
started_utc ended_utc minutes turns_run steps_total timeout usd_lo usd_hi calls writer_usd_hi writer_calls rate_limited hook_failures
gate_blocks oracle{exit,line} final_pass later_kept break_found valid invalid_reason manifest`

* `final_pass`: the oracle exited 0. `later_kept`: no failed check in the `later` group. `break_found`: a check from `task.risk.checks`
  still fails in the final state (null for tasks without risk checks). `valid` rows always have an `oracle`.
* `steps_total` counts `model_completed` events, `calls` counts `model_response_created`; `gate_blocks` counts blocked Stop hooks;
  `hook_failures` counts hook runs that failed or timed out; `rate_limited` counts 429 evidence in the session logs.
* Extra fields: `retryable` (false for `held_out_violation`, `inject_failed` and fatal errors, which are never re-run), `hooks_ok`,
  `hooks_total`, `hook_events`, `sessions`, `engine_state_files`, `turn_log` (per turn: exit, ms, kind, steps, usd), `inject_applied`, `bit`,
  `bit_failed`, `leftover_killed`, `writer_wait`.
* Scrubbed, gzipped session logs of every attempt are saved in `<out>/episodes/<task>__<arm>__r<rep>__a<attempt>/`.
