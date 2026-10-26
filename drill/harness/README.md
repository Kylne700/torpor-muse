# Disaster-drill harness

Runs the pre-registered drill of `drill/DISASTERS.md` and `drill/PREREG.md` (tag `drill-prereg-v1`) in **replay mode** and turns the run records into the published tally. Zero dependencies: Node 20 or newer (tested on 20.19 and 24.18) and POSIX sh. Linux only, because the sandbox is bubblewrap.

**The scored runs are started by the lead only.** Every real disaster id (D01 to D16) needs `--scored`. `--plan` and `build-check` never execute a script step. The harness was built and tested on its own disasters (`test/DISASTERS-TEST.md`, ids D00 to D00i) and on nothing else; no D01-D16 script step has ever been executed (see "Self-test").

## Use

```
node drill/harness/run.mjs --plan [IDS] [--verbose]     parse DISASTERS.md, check the scripts, print each step per arm; executes nothing
node drill/harness/run.mjs build-check [IDS]            run the builders in sandboxed run folders, check them, delete the folders
node drill/harness/run.mjs run ID --arm N --rep R --out DIR [--attempt N] [--control NC1|NC2] [--scored]
node drill/harness/run.mjs batch --out DIR [--only IDS] [--reps N] [--negative-control] [--scored]
node drill/harness/tally.mjs DIR_OR_RUNS.jsonl... [--disasters F] [--reps 3] [--out F]
node drill/harness/selftest.mjs [--real --engine PATH --adapter PATH]
```

Scored run, in order (the lead):

1. Build the adapter from main: `cargo build --release --locked` in the checkout. Arm 4 uses `target/release/torpor-muse` unless `--adapter PATH` is given, so it is whatever main has at that moment; the record keeps its sha256, git head and whether `src/` was dirty.
2. `--plan` (expect `parse errors: 0; static check defects: 0`) and `build-check` (expect `16/16 ok`).
3. `batch --out DIR --scored --deny-contract C --engine "$ENGINE" [--muse-bin "$MUSE"]` (`C` is `binary` or `strict` and is yours to choose once, before the first scored run, because it is part of the replay mode that PREREG section 3 fixes: a scored run is refused without it, and a different value on resume is a changed header; `$ENGINE` is the pinned engine binary, which `--engine` or `TORPOR_BIN` must name: there is no default; `$MUSE` is the Muse launcher, optional, recorded only), then `batch --negative-control --out DIR --scored --deny-contract C --engine ...` (same `--out`, same `C`). The order is the one fixed in PREREG section 2; for each disaster repetition 1 of arms 1 to 4, then 2, then 3. Re-running the same command resumes: finished slots are skipped. `run-log.json` in DIR holds the invocation's header (binaries, harness hash, deny contract, poll values); a changed header refuses to resume (use a fresh DIR).
4. `node drill/harness/tally.mjs DIR --out tally.md`.

Exit codes of `run.mjs`: 0 ok, 1 a check or a run failed, 2 usage, 3 refused (the `--scored` lock, no usable sandbox, a changed run-log header). Options worth knowing: `--engine PATH` (or `TORPOR_BIN`; the pin is checked against `engine.lock` and recorded), `--adapter PATH`, `--muse-bin PATH` (record only: its sha256 and `--version` line, the latter run inside the sandbox), `--deny-contract binary|strict` (see below), `--keep` (keeps the run folder), `--tmp-base DIR`. Test-only and refused for D01-D16: `--no-sandbox` (CI only), `--poison`, `--test-drop-canary`, `--gate-polls N`, `--gate-sleep-ms N`, `--retry-sleep-ms N`.

## What replay does

Per PREREG section 3: the harness builds Muse-shaped hook payloads (`contract.mjs`, tool names `bash`, `edit_file`, `write_file`, `transcript_path` null, `permission_mode` set), runs the arm's real hook commands, honours the answers as Muse would, and runs the scripted calls itself. It starts no Muse and calls no model.

- Run folder: `mktemp -d /tmp/drill.XXXXXX` plus the canary, built by `workspaces/<id>/build.sh` (its own guard), `.drill-run` marker, `.synthetic` checked. Only a folder made by this process and named `drill.XXXXXX` is ever deleted (`sandbox.mjs` `safeDelete`).
- Event order: SessionStart, then per turn UserPromptSubmit, then per call PreToolUse, the call, PostToolUse (or PostToolUseFailure), PostToolBatch, then Stop (a block gives a second Stop with `stop_hook_active` true, at most 8 times), then SessionEnd. A denied call fires nothing after its PreToolUse (ASSUMED). Each call is its own batch (ASSUMED).
- Arm 2 makes the auto-commit (`git add -A`, `commit --allow-empty`) before every turn. R1 is taken before the PreToolUse of the first call that matches `trigger:` (never in a `setup` turn), POST right after that call (or after the deny), FINAL after the fixed recovery of DISASTERS.md section 2. The held-out precondition check runs at the first Stop of the turn named in `precondition:`, before any continuation.
- Calls: `bash` runs `sh -c` in `T/ws` with `T/bin` first on PATH and `CLAUDECODE` unset (NOT VERIFIED whether Muse sets it); `edit_file` needs the find text exactly once (else a tool failure, as ASSUMED for Muse) and `write_file` takes its content from `$ASSETS`/`$SHARED`; a path that resolves outside the run folder (symlinks followed) is refused and the run is INVALID with no retry. `typed [arms 3 4]:` runs in its own shell with the same environment; `torpor promise block N` is followed by the readiness poll of DISASTERS.md section 5 (12 polls, 5 s apart). A gate that never shows `blocking` ends the run as GATE-NOT-REACHED (counted MI, no recovery). `between:` runs in every arm; `C0` in it is the builder's base commit.
- Hooks run with a Muse-like cleared environment (`HOME` in the run folder, PATH with the Node directory appended), cwd `T/ws`, stdin the payload, the timeout of their settings entry; at the timeout the process is killed and so is every leftover that carries the call's `DRILL_CALL` marker in its environment (a timeout is a failed hook that fails open).
- Outcomes: `EX/PA/NR/PV`, `CA/MI`, `FL/PS` per the oracles, plus REFUSED, NOT-TRIGGERED (live only, never produced here), INVALID-PRECONDITION, GATE-NOT-REACHED, INFRA-FAILURE (an attempt that is re-run, at most 2 times; the third failure is INVALID), INVALID (a guard refusal or a third infra failure) and NOT-RUN. D15 (it needs a real `/compact` and a real model, and is the only disaster that does) is NOT-RUN in replay, 12 slots, and the tally shows its cells as UNMEASURED with the reason.
- INVALID-PRECONDITION also covers "the disaster did not happen": arm 1 only (it has no protection), files class when the trigger call lost nothing (`no-loss=yes`, no deny), regression class when no held-out check failed after the disaster. The would-be code is kept in the record (`would_be_outcome`).

## Arms

In this README `<T>` is the run folder and `<H>` its HOME, the folder `home` inside it.

| Arm | What is installed |
|---|---|
| 1 none | nothing; recovery `git reset --hard C0`, then `git stash pop` if the stash is not empty |
| 2 git | auto-commit before each turn; recovery `git reset --hard A` (A = the commit before the disaster's turn); D11 and D12: final auto-commit, `git revert --no-edit` of the commit made before turn 2 |
| 3 engine hooks | `T/bin/torpor` (copy of the binary, sha256 logged), settings with the engine's own 10 events (SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, PostToolUseFailure, Stop, StopFailure, SubagentStop, PreCompact, SessionEnd; the timeouts of its `hooks/hooks.json` at tag `pre-event-final`), no matcher, payloads unchanged. Command: `env TORPOR_HOME=<T>/torpor-home TORPOR_KEY_FILE=<H>/.config/torpor/key <T>/bin/torpor hook <Event>`. PostToolBatch, SubagentStart and PostCompact have no hook in this arm because the engine registers none |
| 4 adapter + engine | `T/bin/torpor-muse` and `T/bin/torpor` (copies), settings written by the adapter itself: `torpor-muse init` run under the sandbox with `HOME=<T>/home`, `TORPOR_HOME=<T>/torpor-home`, `TORPOR_BIN=<T>/bin/torpor`, so the file is `<H>/.config/muse/settings.json` |

Arms 3 and 4 recover with `torpor back [--with-data] --id P` (P = the newest save point after the PreToolUse hooks of the trigger call) or, for D11 and D12, `torpor undo 1`. `torpor log --json` has `point: null` on turn rows, so the harness finds the ids with `torpor show --id N --json` for N = 1, 2, ... until three in a row are missing (`arms.mjs` `PROBE_SH`).

**Arm 4 interface, exactly** (adapter at main 26d8ed8, `torpor-muse 0.1.0`): `torpor-muse init` writes 13 events (PostCompact, PostToolBatch, PostToolUse, PostToolUseFailure, PreCompact, PreToolUse, SessionEnd, SessionStart, UserPromptSubmit, Stop, StopFailure, SubagentStop, SubagentStart); the tool matcher `bash|edit_file|write_file` on PreToolUse, PostToolUse and PostToolUseFailure; SessionStart matcher `startup|resume|clear|compact|fork`; every command is `env TORPOR_HOME=<abs> TORPOR_BIN=<abs engine> <abs torpor-muse> hook <Event>`. The key file is the engine's default under `HOME`, which is inside `T`, so it is the same place as in arm 3. The settings file the harness parses is the one `init` wrote; nothing is hand-made. The T3 brief says the command is `env TORPOR_HOME=<abs> torpor-muse hook <Event>`; main also writes `TORPOR_BIN=` and absolute paths (see AMENDMENTS-PROPOSED.md). Since S3b, `hook UserPromptSubmit` also calls `suggest::trigger` (`src/hook.rs:226`, `src/suggest/mod.rs:487`), which starts a detached run only when the config under `HOME` names a key file; the drill's `HOME` has none, and the network is off, so nothing starts (read in the source and seen in the real arm-4 runs; no key is ever used).

Negative control (`batch --negative-control`): the arm 4 configuration with every hook answer replaced by `{"torpor_drill_poison":1}` (exit 0). The real hook still runs; the tap row keeps what the hook really said in `real_stdout`. NC1 is D06 (expected not PV), NC2 is D16 (expected MI). The tally prints `DRILL INVALID` if NC1 shows PV or NC2 shows CA.

## Which hook answers count (the deny contract)

`contract.mjs` `RULES` is the table, one line per rule, each with a status: **G0** seen in the recorded Muse fixtures (research/59), **SHIM** from Muse facts given by the lead (an input over 256 KiB is delivered; on UserPromptSubmit plain text or noise before the JSON becomes plain context, on Stop it is ignored; a rejected output fails open), **BIN** read in the Muse binary's validation code, **DOCS** only documented, **ASSUMED** the harness's choice. The record lists every rule with its status and which unverified ones the run used.

- `--deny-contract binary` (the default for test runs; a scored run must name it; one value per batch, written in `run-log.json`): only a PreToolUse answer with `hookSpecificOutput.permissionDecision: "deny"` and a non-empty `permissionDecisionReason`, exit 0, stdout starting with `{`, no unknown key, under 16384 bytes stops the call. The legacy `decision: "block"` on PreToolUse is a failed hook (the call runs), exit 2 on PreToolUse is not honoured, `allow` without `updatedInput` is failed.
- `--deny-contract strict`: the literal brief rule, a case the G0 fixtures did not test is not honoured, so a PreToolUse deny is never honoured. It exists only so the lead can see what the drill says under that reading.
- Not emulated: Muse writes `hook_run_terminal` records (status `failed`) into its `session.jsonl`; the harness does not, so only `torpor-muse doctor` would notice it. The emulated session log (`<H>/.local/share/muse/sessions/...`) holds a `runtime.user_intent.accepted` line per prompt and the accepted additional context, the only kinds the adapter reads; Stop block reasons are not logged.

## Sandbox

Everything a run executes (builders, calls, hooks, typed and between commands, recovery, held-out checks) goes through `sandbox.mjs` `exec`: `bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /home --tmpfs /tmp --tmpfs <each real home> --tmpfs /run --dir /run/user --bind T T --unshare-net --unshare-pid --unshare-ipc --die-with-parent --new-session --clearenv`, plus read-only binds for the asset folders (and for the Node install when it sits under /home, /tmp or /root), plus a folder inside `T` bound over `~/.local/share/muse` (Muse writes `session-name-authority/session-names.db` there even with `HOME` changed; replay starts no Muse, so this only matters for a future live mode, which must also pass the Muse install folder as a read-only bind). `HOME`, `TMPDIR`, `TORPOR_HOME` and `TORPOR_KEY_FILE` are inside `T`. No usable bwrap means refusal (exit 3); `--no-sandbox` is for CI and is refused for D01-D16. The oracles run on the host and only read the run folder.

Found on this host and closed: `/run/user/<uid>` (docker.sock, D-Bus session bus), `/run/libvirt/libvirt-sock` (mode 666) and `/run/dbus/system_bus_socket` were reachable without a network. `ls -A /run/user` is empty in the sandbox now and the real home shows only the bound paths (asserted in the self-test, mutation-tested). Not covered: `/mnt`, `/media`, `/opt`, `/srv` stay readable (read-only); `/sys` is host-wide (the self-test reads `/proc/net/dev` to see that only `lo` exists); the host kernel is shared.

Muse itself was only started during the echo-mode investigation, always inside this sandbox.

## Echo mode: verdict NO

Question: can `muse exec --provider echo` issue scripted `bash`/`edit_file`/`write_file` calls through Muse's real tools with the hooks firing? No, found in the 45 minute box:

- Muse 1.4.13 under the sandbox (no key) runs `muse exec --provider echo "run bash: echo hi > a.txt"` with exit 0 and prints `echo: run bash: echo hi > a.txt`; no tool call, no hook event, no file `a.txt` (rerun for this README: the run folder held only `.drill-canary home muse-share tmp`).
- `muse exec --help` lists no script, fixture or tool-call option (`--provider echo|meta`, `--preset`, `-c KEY=VALUE`, `--prompt-file`, `--json`, `--max-model-steps`, ...); the strings of the binary and the wire schema I could read hold no way to hand the echo provider a tool invocation.
- `run.mjs --mode echo` therefore exits 2 ("not implemented"); the tally still supports a second tally over cells whose records say `mode` echo or live, which is empty for this harness.

Not tried, so NOT VERIFIED: driving Muse over its stdio/MSP protocol with a hand-made session, or a fake model server on loopback (needs a model endpoint override and a sandbox with a loopback listener: possible future work, not echo mode).

## Records and tally

`runs.jsonl`: one JSON line per attempt (the schema is `torpor-drill-run/1`), nothing dropped, side files (tap, manifests, held-out output, save points) in `runs/<key>/`. PREREG section 9 fields: `disaster`, `arm`, `repetition`, `mode`, `attempt`, `outcome`, `oracle.line`, `tap`, `engine` / `adapter` / `muse` (paths, sha256, version lines, git head and dirty flag of the adapter, `matches_lock` for the engine), `started_at`, `ended_at`. Also kept: trigger, denied, precondition, the three manifest summaries, facts (C0, auto-commits, P), recovery steps with their exit codes, the contract rule statuses and the unverified rules used, the gate polls, warnings, notes, a trace, the arm's settings file and the output of `init`. Times appear only in `started_at`, `ended_at` and the trace/tap `t` fields.

`tally.mjs` implements PREREG section 7: a cell is the majority of its valid repetitions; 1-1-1 or two differing valid repetitions are a split (counted as a tie, listed); fewer than 2 valid repetitions is UNMEASURED (in no count, listed); ranks NR<PA<EX<PV, MI<CA, FL<PS; REFUSED, NOT-TRIGGERED, INVALID-PRECONDITION, INVALID and NOT-RUN are left out and listed with their reason; GATE-NOT-REACHED counts as MI. Output (markdown): the cells with every repetition in brackets, the four tallies (arm 4 against arm 1, arm 2, arm 3 and the better of arms 1 and 2; the last one needs both cells measured), the echo/live-only tally, prediction hits and misses (against the section 3 table, each miss with its oracle line), the red cells (arm 4 ties or loses to the better of arms 1 and 2, or is itself PA, NR, MI or FL; arm 3 ties are listed separately; expected red cells are marked), the negative control verdict, and every excluded run.

## Assumptions the harness made (all also in the code comments)

1. Arm 3 "every event" = the engine's own 10 events, no matcher (DISASTERS.md A1 says the final harness defines it).
2. Hook failure of any kind fails open; a rule the G0 fixtures did not cover is marked and, in the `strict` contract, not honoured.
3. After a denied call no Post event is fired; each call is its own PostToolBatch; `edit_file` fails unless the find text occurs exactly once.
4. A settings file Muse would refuse to start with (malformed, wrong `schema_version`) is an infra failure of the run.
5. A missing workspace root at a manifest point (a deleted folder) is an empty manifest.
6. `umask 022` for everything the harness starts (the files oracle compares modes).
7. The save point P is read once, at the trigger call, after the PreToolUse hooks and before the call.

## Self-test

`node drill/harness/selftest.mjs` (about 7 s; 186 asserts with `--real`, 15 s). Linux only (prints `SKIP (Linux only)` elsewhere). It checks: the parser on all 16 blocks and on the test list; the predictions snapshot (hand-copied from DISASTERS.md section 3: 10/6/0, 7/8/1, 4/12/0, 7/8/1); the contract rules; the guards (a path escaping the run folder, a missing canary, no usable sandbox, the `--scored` lock through the pure `refusal()` and the CLI without `--scored`, safe deletion); the sandbox hardening; the expected code per arm for the test disasters (a mock engine and adapter in `test/mock.mjs`); the poison wrapper turning a deny into not-PV and a block into MI; a batch, its resume and a changed header; the record fields; the tally on hand-made records (majority, splits, UNMEASURED, GATE-NOT-REACHED as MI, infra replacement, NC1/NC2); that every record names only D00* ids; that no `drill.*` folder is left. If bwrap is not usable (CI runners often forbid user namespaces) the runs fall back to `--no-sandbox` and the sandbox checks print SKIP. `--real --engine PATH --adapter PATH` also runs the real binaries on three test disasters and prints what it saw.

Proof that no real disaster was executed: `--plan` over D01-D16 gives `disasters: 16; parse errors: 0; static check defects: 0`; `build-check` builds the 16 workspaces and nothing else; every record file of the development sessions lists only D00* ids; no `/tmp/drill.*` folder is left; the lock refuses a D01-D16 id without `--scored` before building anything.

Mock versus real: the mock engine is not the engine. The real binaries behaved as follows on the test disasters (engine 2.3.1, adapter 0.1.0 at 26d8ed8): D00 (rm of a tracked file) arms 3 and 4 EX; D00e (undo) arm 3 NR ("Turn 1 changed no files, so there is nothing to undo") and arm 4 EX; D00i (a small HTTP app and a real-shaped promise) arm 1 MI, arms 3 and 4 CA (the Stop gate fired, the agent continued).

## Not verified

Deny behaviour in a real Muse (the contract is from the fixtures and the binary); the real engine and adapter on Muse-shaped payloads beyond the three test disasters; the exact format of `torpor promise list` beyond what was observed; `torpor undo 1` numbering for D11 and D12 under their real builders; the real engine on D01-D16 themselves; the GitHub runner behaviour (bwrap, sqlite3) of the CI step.
