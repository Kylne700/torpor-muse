# Eval runner

Plays every episode (task x arm x rep) through real Muse Code turns, scores it with the held-out oracle, and writes one JSON row per
attempt. The tasks and oracles are described in `eval/DESIGN.md`, the rules of the experiment in `eval/PREREG.md`; this file is about the
runner in `eval/run/`.

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
Exit codes: 0 finished, 2 bad flags or setup, 3 stopped early (cap, fatal error, signal), 130 killed by a second signal.

* The plan is seeded: groups in (rep, task) order, the arms inside a group in an order drawn from the seed and the group alone, recorded as
  `order_index`. The same flags give the same plan; `plan.json` in the out folder refuses a different plan or run id on resume.
* Resumable: rerun the same command. Finished episodes are skipped, rows are written atomically (`rows/<task>__<arm>__r<rep>__a<attempt>.json`).
* **The pilot and the main run are never pooled.** Each gets its own `--run-id` and its own `--out` folder; a folder holds one run id (a
  second id is refused), so the analysis, which reads one folder, cannot mix them. ECHO rows have model `echo` and are not results.
* The analysis reads either `<out>` or `<out>/rows`: `node eval/analysis/analyze.mjs --rows <out>` (plan.json and summary.json carry no
  row field, so they are skipped; the selftest runs the analysis on a runner folder).

## Arms (one function each in `arms.mjs`; the row's `manifest` records what was installed, with the settings file itself)

* **A vanilla.** Nothing: no hooks, no instruction file, not even a settings.json.
* **B notes file.** `AGENTS.md` in the workspace root. It never mentions Torpor. Exact text (the selftest keeps this copy equal to the code):

      # Project notes

      Keep a file called NOTES.md in the project root.

      - Whenever the user tells you that something works, add a line to NOTES.md that says what works and how to check it (a command to run or a request to make).
      - Before you tell the user that a task is done, go through NOTES.md and check every entry again. If one of them no longer works, fix it first, or say so plainly.
      - Keep NOTES.md short and current: remove entries for behaviour that was removed on purpose.

* **C engine hand-wired, no adapter.** The file `$HOME/.config/muse/settings.json` of the episode (`{"hooks": {...}, "schema_version": 1}`)
  holds the engine's ten events and nothing else: no matcher and no translation, exactly arm 3 of the disaster drill (PREREG line 21: "the
  same wiring"; `drill/harness/arms.mjs` writes the same file, and the selftest compares the two byte for byte). Each event is one group
  `{"hooks": [{"command": "env TORPOR_HOME=<episode>/torpor-home TORPOR_KEY_FILE=<episode HOME>/.config/torpor/key <engine> hook <Event>",
  "timeout": N, "type": "command"}]}`, with the timeouts of the engine's own `hooks/hooks.json` (the selftest keeps this list equal to the code):

      SessionStart         timeout 10
      UserPromptSubmit     timeout 10
      PreToolUse           timeout  5
      PostToolUse          timeout  5
      PostToolUseFailure   timeout  5
      Stop                 timeout 45
      StopFailure          timeout 10
      SubagentStop         timeout  5
      PreCompact           timeout  5
      SessionEnd           timeout  2

  Muse uses the same event names, so the mapping is the identity; it lives in ONE place, `ENGINE_EVENTS` and `armCSettings` in `arms.mjs`.
  The engine's own `hooks.json` also has the matcher `Bash|Edit|Write|MultiEdit|NotebookEdit` on the three tool events; the drill leaves it
  out, so C does too: the tool events fire for every tool call of Muse, reads and searches included (with the matcher, the first
  measurements below fired for `bash`, `edit_file` and `write_file` only). `node eval/run/arms.mjs --print-arm-c` prints the file with the
  placeholders `ENGINE_BIN`, `EPISODE/torpor-home` and, for the key, `EPISODE/` plus the episode's HOME and `.config/torpor/key`, and its
  sha256 (`shape_sha256` in the manifest). The engine binary is the one `engine.lock` pins (linux-x64 asset, `torpor 2.3.1`): every C and D row
  records `engine_sha256` and `engine_lock_match`.
* **D adapter + engine.** `torpor-muse init` run inside the episode HOME, built from this checkout (`adapter_commit`, `adapter_sha256`,
  `adapter_version` in the manifest), engine state in `<episode>/torpor-home`. What `init` wrote (13 groups, measured; the exact file is
  `manifest.settings` of every D row, `settings_sha256` its hash); each command is `env TORPOR_HOME=<episode>/torpor-home
  TORPOR_BIN=<engine> <adapter> hook <Event>`:

      SessionStart         timeout 10  matcher startup|resume|clear|compact|fork
      UserPromptSubmit     timeout 10
      PreToolUse           timeout  6  matcher bash|edit_file|write_file
      PostToolUse          timeout  6  matcher bash|edit_file|write_file
      PostToolUseFailure   timeout  6  matcher bash|edit_file|write_file
      PostToolBatch        timeout  5
      Stop                 timeout 35
      StopFailure          timeout 10
      SubagentStart        timeout  3
      SubagentStop         timeout  5
      PreCompact           timeout  5
      PostCompact          timeout  3
      SessionEnd           timeout  5

  **The writer in arm D** (the model call that proposes checks): ON when the command has `--writer-key-file FILE` (keyed runs only; the file
  must be mode 0600). The runner then writes `$HOME/.config/torpor-muse/config` in the episode HOME with `key_file = FILE` and
  `day_cap_usd = <--writer-day-cap>` (default 0.05, a fresh ledger per episode, so 0.05 per episode), and binds FILE read-only into the model
  sandbox (so a model that looks can read it; saved session logs are scrubbed of the key). The writer uses the adapter's default model
  (standard prices, not the contributor id). Without `--writer-key-file`, and always in ECHO runs, the writer is OFF. Every D row says which
  (`manifest.writer` on/off, `writer_day_cap_usd`, `writer_model`) and records the writer's own calls and dollars apart in `writer_calls` and
  `writer_usd_hi`, read from the writer's ledger (`<episode>/torpor-home/muse/suggest/ledger.jsonl` since the writer state fix 1b01d36;
  an older adapter keeps it under `$HOME/.local/state/torpor-muse`, and both places are read). The writer's dollars count against `--cap-usd`.
  **The writer is detached and nothing outlives a sandbox, so with the writer on the launcher waits for it after every turn** (`--drain`, up
  to 60 s, `manifest.writer_drain_s`): the writer of turn n has finished before turn n+1 starts, which a live session would not guarantee.
  That wait is not the model's time: the turn limit is extended by it and the 15 minutes do not count it. A writer still running when the
  wait ends is stopped and counted. Per turn: `turn_log[].drain_ms` and `drain_cut_off`; per episode: `writer_wait` (`waited_ms`,
  `still_running`). Keyed runs with the writer off use `--drain 0`; `--net none` calls take no `--drain` (nothing outlives them). **NOT VERIFIED** until the exploratory episode below has run: that the writer
  fires through Muse's hooks in this setup.

## Every Muse Code call

One call type, `muse exec`, once per turn (and `muse --version` once per run). Keyed:

    muse exec --provider meta --model <id> --reasoning-effort medium --contributor-mode off --approval-judge off \
        --disable-web-tools --disable-reminders --trust-workspace --disable-approval --max-model-steps 40 \
        --session-id <uuid> --prompt-file <file>

ECHO: `--provider echo` and no `--model` or `--reasoning-effort`. The session id is the same for the six turns of an episode and new for
every episode. The environment is `HOME`, `TMPDIR`, `XDG_*` inside the episode folder, `MUSE_NO_AUTO_UPDATE=1`, a fixed `PATH`, and in keyed
runs `META_API_KEY` (nothing else is passed in). The flags are also in every row's `manifest.flags`.

## One episode

1. A fresh folder under `--tmp`; `make-workspace.mjs <task> ws` (start state, git repo, `.synthetic` marker); HOME, TMPDIR and TORPOR_HOME inside it.
2. Turns 1 to 6 from `task.json`: each is one `muse exec` as above inside the sandbox.
   **Multi-turn mechanism (measured, ECHO and keyed):** a second `exec` with the same session id continues the conversation (a keyed nonce
   given in turn 1 was repeated in turn 2). The fidelity gap: every exec fires SessionStart (`startup` in turn 1, `resume` after),
   UserPromptSubmit, Stop and, on exit, SessionEnd, so a six-turn episode has five extra resume boundaries that one interactive session
   would not have. The SessionEnd run is not written to `session.jsonl`; hook counts cover SessionStart, UserPromptSubmit, the tool events,
   PostToolBatch and Stop.
3. Held out: `task.json`, the oracle, the patches, results and other episodes are never inside a model sandbox (only the episode folder is
   bound read-write). The turn prompt is written into the episode just before the turn and removed after it. After every turn the episode
   folder is scanned (by name and by content hash) for held-out files; a hit makes the row invalid (`held_out_violation`, not re-run).
4. Injected tasks: after turn 3 the host applies `inject.patch` (`git apply --recount -C1 --whitespace=nowarn`), runs the oracle once and
   records `bit`: a `risk.checks` check failed. A patch that does not apply is `inject_failed` (invalid, not re-run). A patch that applied
   but did not bite stays a valid row with `bit: false` (PREREG section 6).
5. Score: run `node eval/oracles/<app>.mjs --task <id> --dir <ws>` in a sandbox with no network (a writer-on turn has already waited for its
   writer, see above). Caps: 6 turns, 15 minutes wall clock (a timeout counts as a fail: `final_pass` and `later_kept` false, the oracle still recorded).
   A failure of Torpor itself (a hook that exits non-zero or times out) is a result: the row stays valid and `hook_failures` counts it.
6. Infra failures (sandbox error, Muse crash, oracle exit 2, 429 after the global backoff, an adapter `init` that fails) make the row
   invalid with a reason; the episode is re-run in a fresh workspace up to 2 more times (3 attempts in all), every attempt keeps its row.
   A turn that hits `--max-model-steps` is the model's failure, not infra.

## Whole groups, stops and invalid rows

* A group is one (task, rep) with all its arms. The runner plays **one group at a time**: its arms run together (up to `--concurrency` at
  once; one after the other in the seeded order when that is 1), and the next group starts only when the previous one is finished. Before a
  group starts, the cap must leave room for all of it: spent plus the dearest finished episode (or 6 x the per-turn floor before the first
  one) for every episode in the group. So **a forced stop leaves complete pairs**: a stop (cap, fatal error, first SIGINT or SIGTERM)
  starts no new group and lets the one in progress finish. The exceptions, said plainly: a group whose episodes cost more than the
  estimate can still hit the cap inside it (the interrupted episode gets a row `spend_cap`, the rest of the group is not started); a second
  signal, a crash or a power cut kills what runs; a fatal error (rejected key, unknown model id) ends the run at its first episode.
  The same command finishes a half group. `summary.json` has `groups_planned`, `groups_complete` (every arm of the group has a valid
  row) and `stopped_early`.
* An invalid row has `valid: false` and a non-empty `invalid_reason` that starts with a word: `infra`, `rate_limited`, `muse_exit_<n>`,
  `sandbox_refused`, `adapter_init_...`, `held_out_violation`, `inject_failed`, `spend_cap`, `fatal`. Every invalid row is one attempt: the
  runner writes at most 3 rows per episode (the analysis refuses more).
* **Rows that must not be re-run** have the field `retryable: false` (`held_out_violation`, `inject_failed`, and every valid row). The
  runner never re-runs them, and the analysis reads the field: a unit with such a row and no valid attempt is `exhausted` at once (excluded,
  counted), not pending. PREREG section 6 lists `inject_failed` as invalid, not re-run, counted and listed with its reason, because after
  turn 3 the files differ per arm and a patch that no longer applies is the arm's own doing.

## Sandbox

Everything the model touches (and the oracle, patch and init runs) goes through `scripts/sandbox/muse-sandbox.sh` (v3; its README has the
details). `sandbox.mjs` only builds the argument list (`--run-dir`, `--ro`, `--net none|meta`, `--allow`, `--chdir`, `--drain`) and never
runs bwrap itself. The launcher clears the environment (only `TORPOR_*` and, with `--net meta`, `META_API_KEY` pass in), puts HOME, TMPDIR
and `XDG_*` inside the episode folder, hides the real home, always has a private pid namespace, and gives no network under `--net none`.
Keyed runs use `--net meta`: only `api.meta.ai:443`, through a host proxy that logs every connection. That makes keyed runs safe at any
`--concurrency`. Nothing outlives a sandbox call except the wait for the writer described above.
The proxy's log of an episode is `<episode>/net/proxy.log` (hidden from the model). The runner reads it at the end of the episode into the
row's `egress` (`allowed`, `denied`, `via_fwd`, and `other`: up to 8 distinct lines that are not an allowed `api.meta.ai:443`) and copies it
to `<out>/episodes/<key>/proxy.log`. **It records and does not stop**: a non-empty `other` is for the operator to read. `via=fwd` lines come
from hooks and the writer, `via=proxy` from Muse, so `via_fwd > 0` in a writer-on row is a second sign, next to the writer's ledger, that the
writer reached the API. The row's `manifest.sandbox` has the launcher's commit, whether it was modified, and the sha256 of its two files.

## Spend

After every turn the runner sums `model_completed.usage` from the episode's session logs plus the writer's ledger lines, at Meta's list
prices (contributor ids: cached 0.002, input 0.10, output 0.20 USD per 1M tokens; standard: 0.15, 1.25, 4.25). `usd_lo` counts cached tokens
once at the cached price; **`usd_hi` also counts them at the input price and adds reasoning tokens to output**, so `usd_hi` is the bound the cap
uses (`writer_usd_hi` is the writer's own ledger, summed on top for the cap). Before each turn the runner refuses to start if spent plus the
dearest turn so far (once per running turn) would pass `--cap-usd`. `spend.jsonl` in the out folder has one line per turn and per late
writer charge. Rate limits: a 429 or Retry-After from Muse pauses every worker (the pause is not counted in the 15 minutes, nor is the writer's wait).

## Row (schema 1; fields are added, never renamed)

`schema run_id task app kind torpor_should_help arm rep attempt seed order_index model muse_version engine_version adapter_commit
started_utc ended_utc minutes turns_run steps_total timeout usd_lo usd_hi calls writer_usd_hi writer_calls rate_limited hook_failures
gate_blocks oracle{exit,line} final_pass later_kept break_found valid invalid_reason manifest`

* `final_pass`: the oracle exited 0. `later_kept`: no failed check in the `later` group. `break_found`: a check from `task.risk.checks`
  still fails in the final state (null for tasks without risk checks). `valid` rows always have an `oracle`; invalid rows have null outcomes.
* `steps_total` counts `model_completed` events, `calls` counts `model_response_created`; `gate_blocks` counts blocked Stop hooks;
  `hook_failures` counts hook runs that failed or timed out; `rate_limited` counts 429 evidence in the session logs.
* Extra fields: `retryable`, `hooks_ok`, `hooks_total`, `hook_events` (completed runs per event), `sessions`, `engine_state_files`,
  `turn_log` (per turn: exit, ms, kind, steps, usd, and for a writer-on turn `drain_ms`, `drain_cut_off`), `inject_applied`, `bit`,
  `bit_failed`, `leftover_killed`, `writer_wait` (null unless the writer is on), `egress` (null in ECHO runs).
  `manifest` has the arm's install (with `settings` for C and D), `flags`, `sandbox`, `mode`, `max_steps`, the caps, the versions, and for
  C and D `engine_sha256` and `engine_lock_match`.
* Scrubbed, gzipped session logs of every attempt are saved in `<out>/episodes/<task>__<arm>__r<rep>__a<attempt>/`.

## Measured with Muse Code 1.4.13 (keyed, contributor id, task notes-ctrl)

The first three bullets were measured with the first arm C wiring (the engine's matcher on the three tool events, no `TORPOR_HOME` in the
command) under the old sandbox (v2.1), before arm C was changed to the drill's arm 3 above.

* The engine's matcher `Bash|Edit|Write|MultiEdit|NotebookEdit` fires PreToolUse for exactly the `bash`, `edit_file` and `write_file` calls
  (arm C: 16 + 5 + 5 = 26 hooks) and not for `read_file` (12); the adapter's matcher `bash|edit_file|write_file` fires the same way
  (arm D: 11 + 7 = 18, `search` and `read_file` not). Whether Muse aliases the names or matches case-insensitively is not determined.
* In arm C the engine's `torpor log` showed all six turns but "no files" for every turn; in arm D it showed the edited files. Not
  re-measured here. The drill's notes give the cause (`drill/DISASTERS.md` A1): the engine's edit list and shell checks use Claude Code's
  tool names (`Edit`, `Write`, `Bash`), Muse's are `bash`, `edit_file`, `write_file`, and only the adapter translates them
  (`src/hook/normalise.rs`). That holds for the arm C of the drill and of this runner alike.
* Errors: a rejected key ends `muse exec` with exit 1 and "failed to fetch model catalog: authentication failed"; an unknown model id with a
  good key gives exit 1 and "model ... does not exist or you lack access", no tokens. Both are `fatal` (the run stops). The 429 message is
  NOT measured.
