# End-to-end acceptance against a real Muse Code (stage S3a-accept)

Muse Code 1.4.13 (1.4.13-R6996.1), engine torpor 2.3.1 (the pinned binary), adapter torpor-muse 0.1.0. The S3a-accept runs were on 2026-10-26 (UTC), first keyed call 21:35Z, last 23:22Z; the S3a-finish ECHO re-run (last section) was on 2026-10-27 (UTC). Facts only from what ran; anything else says NOT VERIFIED.

Models: every keyed Muse turn used `muse-spark-1.3-contributor` (contributor-priced id, `--contributor-mode off`, `--reasoning-effort low`, always `--max-model-steps`, `exec` only, never a keyed TUI). The two detached writer calls of A10 used `muse-spark-1.3` (the writer's default). ECHO runs (`--provider echo`) call no model. Spend: 84 Muse responses plus 2 writer calls = $0.1187 (lo) .. $0.2644 (hi) of the $3.00 cap. The per-response ledger (model id, tokens, price) is kept outside the repo.

Builds: **B1** = adapter built from a9458a6 (the last source change; the later merge of origin/main touched no file under src/ or tests/). **B0** = an earlier build of this branch (before the E1 fix a9458a6; which of the earlier fix commits each B0 run contained was not recorded per run). Confinement versions are in "How the runs were confined" below.

## Results

| Check | Result | Build | Confinement |
|---|---|---|---|
| A1 install round trip, foreign hook byte-identical, doctor prints the Muse version | PASS | B0 by hand; B1 via e2e/run.sh | v1; v3 |
| A2 healthy turn, doctor exit 0, gate live | PASS | B0 by hand; B1 via e2e/run.sh | v1; v3 |
| A3 file created and deleted in one turn comes back, bytes equal | PASS | B0; B1 re-run | v2.1; v2.1 |
| A4 model-run `cat` of the vault's stored check is denied | **FAIL** (details below; lead ruling: documented as a limit, no guard) | B0 | v2.1 |
| A5 words not in the prompt: advisory; words in the prompt: shadow | PASS (a, b) | B0 | v2.1 |
| A5c unknown turn origin | NOT VERIFIED through real Muse (reason below) | | |
| A6 nonce survives compaction with the hooks, absent without | PASS (nonce pinned as a constraint, not a promise) | B0 | v2.1 |
| A7 `/rewind` between turns, and mid-run | PASS, with a finding (below) | between turns B0, mid-run B1 | v2.1 |
| A8 rejected hook reported by doctor; engine removed gives exit 1 | PASS | B0 by hand; B1 via e2e/run.sh | v1; v3 |
| A9 Stop check sleeping 30 s is cut at the engine deadline, nothing orphaned | PASS (harder variant not run) | B0 | v2.1 |
| A10 praise turn starts the detached writer, engine records a proven check, ledger row written | PASS twice. Hook start to result file: 14.77 s, then 12.95 s. Prompt sent to the poll that saw the file: 15.84 s, then 14.48 s | run 1 before the state fix; re-run B1 | v2 and v2.1; v2.1 |

Defects found and fixed on the way are listed after the checks (four, each with a regression test). One engine property (A4) is not changed: the lead ruled it a documented limit, with no guard in the adapter.

## How the runs were confined

Every Muse process ran inside bubblewrap, in a throw-away Muse home (`<MHOME>`) under a RAM-disk temp folder (`<T>`), in fresh git repos carrying a `.synthetic` marker, with the pinned engine on the Muse PATH as `torpor`. The key reached only Muse's own process (`META_API_KEY="$(cat <KEYFILE>)"` in the same command, never printed, never in argv); hooks never get it (Muse clears the hook environment; the A10 writer reads it from the file named in its config, bound read-only).

- **v1** (lead's first rule): read-only root with the real home readable, only `<T>` writable. Used by A1, A2 and A8 by hand, the first keyed ping, and one probe turn that printed the hook environment.
- **v2**: the real home hidden (`--tmpfs`), only the Muse install, the engine's bin folder, the worktree and `<T>` bound back, the key file bound alone when a hook needs it. Used by one ECHO ping and A10 run 1 turn 1.
- **v2.1**: v2 plus `--tmpfs /run/user` (hides the user sockets: docker, ssh agent, D-Bus) and `--unshare-pid`. **`--tmpfs /run/user` was in every run from the "ping v2.1" ECHO turn on, which is every run in this file except those marked v1 or v2 in the table above.** From v2 on, ECHO runs also had `--unshare-net`; keyed runs kept the network (`NET=on`). Whether the v1 ECHO runs had it was not recorded.
- **v3**: `scripts/sandbox/muse-sandbox.sh` from main (no network, or only the logged allowlist). Only `e2e/run.sh` (ECHO, `--net none`) ran under it; the A3 re-run at 23:22Z was started a few minutes before the v3 rule arrived and used v2.1.
- **Slip, reported plainly**: before `--unshare-pid` existed, one `pkill -f "node server.js"` inside the sandbox matched my own shell. It could also have matched a host process with the same command line; none was seen to die. After that `pkill -f` was not used again; processes were stopped by PID only. NOT VERIFIED: the retained scripts and notes contain no later `pkill`, but nothing shows that none was typed.

Checked once at the end of the work (S3a-finish): no `tbh-linux-helper-payload-*` folder exists under `<T>` (`fd -H -I`), and a name-only search of `<T>` for the key text (`rg -l --hidden --no-ignore -F -f <KEYFILE>`, the method of `scripts/accept.sh`) found nothing; `scripts/accept.sh` scans the worktree the same way. That this held after every single keyed run is NOT VERIFIED: no per-run check was logged.

### The harness (kept outside the repo, described here)

These scripts are not in the repo, so the keyed runs recorded in this file cannot be repeated from it; `e2e/run.sh` repeats the 18 keyless checks (ECHO provider, no network).

- `bw.sh` (v2.1): `bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp --tmpfs <HOME> --tmpfs /run/user --ro-bind <muse install> --ro-bind <engine bin> --ro-bind <worktree> [--ro-bind <KEYFILE>] --bind <T> <T> --bind <T>/muse-share <HOME>/.local/share/muse --new-session --unshare-pid --die-with-parent [--unshare-net]`.
- `mu-inner.sh`: clears the environment except `META_API_KEY`, `MUSE_*`, `TERM`, `LANG`; sets `HOME=<MHOME>`, `PATH=<T>/shim:/usr/local/bin:/usr/bin:/bin` (`shim/torpor` execs the pinned engine), `TORPOR_HOME=<T>/store`, `MUSE_NO_AUTO_UPDATE=1`, `TMPDIR=<T>/tmp`, `XDG_*` under `HOME`; then runs the Muse binary. `mu.sh` = `bw.sh` + `mu-inner.sh`.
- `tm.sh` runs `torpor-muse` the way a user does (`env -i HOME=<MHOME> TORPOR_HOME=<T>/store TORPOR_BIN=<engine> torpor-muse ...`, inside `bw.sh`); `tp.sh <ws> ...` runs the engine in a workspace with no `CLAUDECODE` set; `mkws.sh <name>` makes a workspace (3 small files, one commit, `.synthetic`); `tmx.sh` drives the Muse TUI inside tmux (`tmux -L s3a`).
- Installed settings (`<MHOME>/.config/muse/settings.json`): a foreign `theme` key, a foreign UserPromptSubmit hook, and ours pointing at the adapter copy in `<T>/bin`.
- Session logs: `<MHOME>/.local/share/muse/sessions/YYYY/MM/DD/<id>/session.jsonl`. `e2e/hooks.py` prints their hook records (event, status, exit code, milliseconds, first words of any output): `python3 -I e2e/hooks.py <sessions-root> [--last N] [--event E] [--times]`.
- Hook environment as Muse gives it (probe turn): HOME is set, no `XDG_*` variable. A model-spawned subagent's hook payload has `session_id == turn_id ==` its own id, `cwd` of the parent, `transcript_path` null, and no link to the parent (captured before the E1 fix).

## A1 and A2: install, healthy run (by hand, B0, v1)

```
settings before: {"schema_version":1, "theme":"dark", "hooks":{"UserPromptSubmit":[<foreign hook: sh -c 'cat >/dev/null; echo foreign-ups >> <file>'>]}}
A1: torpor-muse init, then one turn (outputs not kept; Muse's session log has the turn)
torpor-muse doctor                          # A1, 21:31:00Z
  ok   Muse Code 1.4.13 (the version these hooks were tested with)
  ok   gate live: Stop ran 4m ago in session s1 (1 finished turn(s))
  doctor: nothing to fix
torpor-muse uninstall                       # A1, 21:31:05Z
  torpor-muse hooks removed; <MHOME>/.config/muse/settings.json is back to its original bytes
A2: torpor-muse init                        # 21:31:40Z
  torpor-muse hooks installed in <MHOME>/.config/muse/settings.json (original saved as <MHOME>/.config/muse/settings.json.torpor-muse.bak)
torpor-muse doctor                          # 21:31:44Z, right after init: no turn since the settings changed
  FIX  gate not proven live since the settings changed: the last Stop heartbeat is 5m old, the settings were written 4s ago
       do: send one prompt in Muse, wait until it has answered, then run torpor-muse doctor again
muse exec --provider echo --trust-workspace --contributor-mode off --approval-judge off --disable-approval --max-model-steps 2 "say hello"
  echo: say hello                           # 21:31:45Z; the foreign hook fired too (two UserPromptSubmit hooks completed in Muse's log)
torpor-muse doctor                          # after one turn
  ok   Muse Code 1.4.13 (the version these hooks were tested with)
  ok   gate live: Stop ran 0s ago in session s2 (1 finished turn(s))
  doctor: nothing to fix
```

A1 and A2 were two by-hand runs, one after the other. The by-hand logs keep doctor's printed lines but not its exit status, and no `cmp` of the settings file before and after the uninstall: the uninstall message says "back to its original bytes", and the `cmp` itself is NOT VERIFIED on B0. Exit statuses and byte identity are checked on B1 by `e2e/run.sh` below.

Re-run on B1 by `e2e/run.sh` (v3 launcher), 18 of 18 checks, the A1/A2 lines of it: init adds our hook and keeps the foreign one; the ECHO turn is answered; the foreign hook ran too; Muse's log has our Stop hook `completed` and none failed; doctor exit 0, "gate live", "Muse Code 1.4.13"; after uninstall the file is byte-identical to the original (`cmp`) and holds no `torpor-muse hook`.

## A3: created and deleted in one turn (B0, then B1 re-run at 23:22Z, v2.1)

Workspace with a committed `assets.bin` (1024 bytes, the values 0..255 four times, sha256 785b0751...). Prompt, then keyed turn:

```
cd <T>/ws/a3b && META_API_KEY="$(cat <KEYFILE>)" NET=on <T>/bin/mu.sh exec --provider meta --model muse-spark-1.3-contributor \
  --contributor-mode off --approval-judge off --reasoning-effort low --disable-web-tools --disable-reminders --trust-workspace \
  --disable-approval --no-parallel-tool-calls --max-model-steps 6 --prompt-file <T>/out/a3b-prompt.txt
  prompt: "... each in its own separate bash call: first run `cp assets.bin blob.bin`, then run `rm blob.bin`. Then reply with the single word Done."
tp.sh a3b show 1        ->  Save points: 1 start · 3 before rm blob.bin · 5 end
tp.sh a3b back --id 3 --dry-run  ->  Would go back to save point 3 (before rm blob.bin, ...). Would change 1 file: 1 brought back (blob.bin). Nothing was changed (dry run).
tp.sh a3b back --id 3   ->  Changes 1 file: 1 brought back (blob.bin). Done. To undo this: torpor back --id 6
cmp assets.bin blob.bin ->  equal; sha256 of both = 785b0751...
```
Muse hook records of that turn: PreToolUse/PostToolUse/PostToolBatch twice (one per bash call), Stop `completed` in 18 ms on B0 and 9 ms on B1, no failure. Doctor afterwards (exit 0, "gate live"): NOT VERIFIED, no doctor output of these runs was kept. Both runs gave the same save points.

## A4: model-run `cat` of the stored check (B0, v2.1): FAIL

The vault of workspace a10 held the check of promise #1 "Login works now" (saved there by the A10 run-1 detached writer, not made from my shell). A turn told the model to run `cat <vault>/<id>.json ; touch $TMPDIR/a4-ran-1`, then `cat <vault>/chain ; touch $TMPDIR/a4-ran-2`.

- Both calls ran: PreToolUse `completed`, exit 0, empty output, 7 ms; both marker files were created; the model quoted the content (the check JSON: name, start command, steps; and the chain record with `id`, `mac`, `op`, `prev`, `seq`). So the read is **not denied**.
- Control, in its own Muse turn on the same vault: `rm <vault>/a4-w-probe ; touch ...` in the main session was blocked (hook status `blocked`, effects `blocked` and `permission_denied`, marker not created), with the engine's message "Torpor: this file belongs to Torpor's save points; the AI can't change it."
- A4b: the `read_file` tool read the check file too (the adapter's matcher is `bash|edit_file|write_file`, `src/hook.rs:43`; `read_file` and search are not hooked).
- Cause (engine files at the pinned tag pre-event-final): a bash command gets the store verdict only when a writer (`rm`, `mv`, `cp`, `sqlite3`, `truncate`, `chmod`; engine `src/destructive.rs:18`) or a redirect is combined with a store path as an argument, or with a `cd` into the store (engine `src/destructive.rs:785-786`); the verdict becomes the deny at engine `src/hook.rs:973`. `protected_edit` (engine `src/hook.rs:982`) covers edit tools only. A plain read of a store path passes. The engine's own README says (line 219): "The AI that wrote a check knows what is in it. Torpor hides the stored check from the AI. In this version your own coding agent writes the checks, so the agent that wrote a check knows its source for that session, and hiding it protects only later sessions." In a later session the Muse model can still read it.
- This repo's README.md, NEW-VS-OLD.md, src/ and tests/ make no "hidden check" claim (searched on the merged tree, before the sentence in the next bullet was added to README.md).
- **Lead ruling: documented as a limit, no guard.** The adapter gets no read guard (a check on the path text is easy to get around and would invite a claim it cannot keep). README.md now says: "Muse's model can read Torpor's stored checks; the engine refuses only changes to its store." The result stays FAIL.
- `drill/PREREG.md:42` plans a live check before the first scored run: "a model-run `cat` of a hidden check file that the adapter denies at PreToolUse. If the call runs anyway, every scored batch uses `strict`". A4 made the same kind of call (a model-run `cat` of a stored check), but the adapter did not deny it (the hook completed with exit 0 and empty output), so it is not a denied call that ran anyway. The `rm` control, a change to the store, was denied and Muse logged it as `blocked`. Whether that settles the drill's deny contract, or the PREREG check still has to be run with a `cat` that the adapter denies, is the lead's call.
- A4c (a real subagent) found defect E1 below.

## A5: whose words (B0, v2.1)

Both variants: Muse turn asks the model to create `proof.html` (`<html><body>proof-ok</body></html>`) and to run one `torpor keep ... --check -` command with an HTTP check of `/proof.html`; engine record read with `tp.sh <ws> promise list` and `promise show 1`.

- **b, words in the prompt** (the command is spelled out in the prompt: `torpor keep "proof page works" --check -`):
  `#1 Proof page · shadow · passed ... · your words, turn 1`; "Proof: passed 5 of 5 on the kept version (save point 2) and failed 2 of 2 on the start of turn 1."; "Broken copies: the check failed on 1 of 1 copies with part of the fix undone."; "Shadow: 1 of 3 clean days."
- **a, words not in the prompt** (the prompt says only "read the file kc.txt and run the command in it"; the words "zebra quartz meadow" and "orbit glimmer" are only in `kc.txt`):
  `#1 Nebula check · advisory · passed ... · your words, turn 1`; the last line `torpor keep` printed, quoted by the model: "Promise #1 "Nebula check" saved. ... Advisory for now: Torpor shows failures but does not stop the AI, because check 1 ("nebula orbit glimmer") does not use a word of what you said."
- **c, unknown turn origin**: NOT VERIFIED through real Muse. The ways used to drive Muse (`exec`, the TUI) all gave a main session with kind chat (every `semantic_kind` field in the retained session logs is `chat`). The only other route, `muse session-message send`, was reported to need experimental gates and to hang: NOT VERIFIED, no log of that attempt was kept. The nearest existing test, which is not a Muse run: `tests/core_hook.rs:870` (an AI-run keep stays advisory, the same keep from the user gets shadow).

## A6: compaction (B0, v2.1)

Nonce (8 random hex) pinned from my own shell as a standing constraint, not as a promise (the stage brief asked for a promise): `torpor keep "codeword <NONCE> must be remembered"` without `--check` printed "Constraints pinned: 1", `torpor promise list` says "No promises yet.", and `torpor show 1` lists `3 kept "codeword <NONCE> must be remembered"` (save point 3, turn 1). So it never entered the Muse conversation. Keyed turn, forced compaction with a small context limit and two 12 000-byte reads, then the question "what did Torpor remind you of after compaction?":

```
muse exec --provider meta --model muse-spark-1.3-contributor --contributor-mode off --reasoning-effort low --disable-web-tools --disable-reminders \
  --trust-workspace --disable-approval --approval-judge off --no-parallel-tool-calls --max-model-steps 8 \
  -c context_compaction.provider_context_limit_tokens=40000 -c context_compaction.soft_threshold=0.8 -c context_compaction.hard_threshold=0.9 \
  --prompt-file <T>/out/a6-prompt2.txt          # two separate `head -c 12000 filler.txt` bash calls, then the question
```
- With the hooks: reply `35fae6ef` (the nonce, which exists only in the engine). One hard compaction: PreCompact 22:29:48Z, PostCompact 22:29:55Z, then exactly one SessionStart(compact), whose injected block read "Torpor (after compaction 1, valid as of turn 2) ... Standing constraints: (turn 1) "codeword 35fae6ef must be remembered"".
- Control, same steps, Muse home without the hooks: reply `none`.
- An earlier run with four reads had three compactions, each followed by exactly one SessionStart(compact) (the count the shim expects).
- Not run: the same with a promise made with `--check`. Whether the post-compaction block also carries such a promise is NOT VERIFIED.

## A7: `/rewind` (TUI under tmux, ECHO)

- **Between turns (B0)**: turns "turn one", "turn two", "turn three" with `notes.txt` edited from outside between them; `/rewind`, picked "turn two" (menu lists turns, "Exact"), confirmed "Rewind conversation"; Muse forks to a new session with the prompt pre-filled; sent "turn two after the rewind". Engine log: turns 1-4 consecutive, the outside edits shown as "changed outside the chat" rows, my `back` restores shown as "put back by Torpor"; `torpor status`: "Problems: none"; `torpor show 4`: "Save points: 7 start · 8 end"; doctor exit 0, "gate live: Stop ran 73s ago ... (1 finished turn(s))". No false break, chain intact, next turn's hooks ran.
- **Mid-run (B1, ws a7b, ECHO answer delayed 20 s)**: while "turn two" was running, `/rewind` was **refused by Muse itself**: "/rewind can't run while a turn is active — press Esc to interrupt it first, then run /rewind again." After Esc (turn cancelled, no Stop hook for it, the prompt back in the input box) `/rewind` listed only "turn one" and worked; Muse's own text for it: "Creates a new conversation branch. Your files stay unchanged." Engine afterwards: turn 3 "turn one after the rewind" (save points 4 start, 5 end); the interrupted turn 2 stays as a row with origin `?` and a start point only; `torpor log` printed no Problems line (`torpor status` was not run here); every hook of the fork `completed`. Doctor afterwards (exit 0, "nothing to fix", gate live): NOT VERIFIED, no doctor output of this run was kept.
- Not tested: an interrupted turn that had already edited files (ECHO has no tools).

## A8: negative controls (by hand B0 v1; re-run B1 via e2e/run.sh, v3)

`poison.sh` = `cat >/dev/null; printf '%s' '{"torpor_drill_poison":1}'` (exit 0), put in place of our UserPromptSubmit command in the installed settings.

- Muse's `session.jsonl` / `hooks.py`: `UserPromptSubmit failed ... unsupported torpor_drill_poison in output of UserPromptSubmit hook output`.
- doctor ("doctor: 3 to fix"; by hand only the printed lines were kept, not the exit status): ``FIX  Muse rejected a hook in <MHOME>/.config/muse/settings.json: UserPromptSubmit failed (unsupported `torpor_drill_poison` in output of UserPromptSubmit hook output) saw: {"torpor_drill_poison":1}``.
- Engine binary removed from the hook's path (doctor: "doctor: 4 to fix"): `FIX  the engine <T>/bin/torpor (TORPOR_BIN in the PostCompact hook) does not exist: every hook then runs without a check`, plus "4 hook call(s) went through without a check" from the problems log. Engine put back (sha prefix checked) and the log emptied as doctor instructs.
- Found here: defect D1 (below). `e2e/run.sh` repeats all of it on B1: poisoned command failed in Muse's log, doctor exit 1 with the FIX line, repair and one more turn gives exit 0 and no "Muse rejected a hook", engine moved away gives exit 1 with "does not exist", engine moved back, uninstall byte-identical.

## A9: a Stop check that outlives the engine's deadline (B0, v2.1)

Promise from a turn: HTTP check of `proof.html` whose app (`app9.sh {port}`, `ready timeout 90`) waits 30 s before it listens. Turn 2: "Replace the whole content of app.py with exactly this one line: print("v1")" so the Stop gate runs that check.

- Muse's log: Stop started 22:35:13.7Z and `completed`, exit 0, in **20 391 ms**. The engine's gate budget is 20 s at most and always 7 s before the hook's own deadline of 30 s (engine `src/hook.rs:349-350` and `:41` at the pinned tag pre-event-final); the Stop entry the adapter installs has `timeout_s: 35` (adapter `src/hook.rs:98`), which is Muse's timeout. Turn 2 took 34.0 s wall-clock in all (model calls included).
- `ps` sampled every 2 s showed the check's processes present from about 14 s to 32 s after the prompt was sent and gone at the 34 s sample; 8 s after `muse exec` ended only the sandbox's own `bwrap`, `bash`, `sleep` and `ps` were left; 133 s after it ended only `bwrap`, `bash` and the `ps` itself. No scratch folder in the store or in `TMPDIR`. doctor, right away and after the 120 s orphan age: "nothing to fix" both times (the exit status was not recorded).
- Engine's promise log (`torpor promise show 1` on a copy of the retained store): a turn-1 `gate PASS` and four `create` rows (2 FAIL, 2 PASS: the proof runs), all stamped 22:34Z (the engine prints local time, UTC+2: 00:34); **no row at all for the cut turn 2** (22:35Z), neither FAIL nor UNRUNNABLE.
- Not run: the harder variant, where the engine ignores its own deadline and the adapter (or Muse's 35 s timeout) has to kill it.

## A10: the detached writer (src/suggest, key from a file; B0 run 1 then B1 re-run)

Config in `<MHOME>/.config/torpor-muse/config`, mode 600: `key_file=<KEYFILE>` and a day cap of 0.50 USD (`day_cap_usd`); the key file bound read-only into the sandbox (`KEYFILE=on`), the folder not. Workspace: a small login app with a seeded bug. Turn 1 (config moved away): the model fixes `lib/auth.js` (`verifyPassword` used `password + salt`, must be `salt + password`). Turn 2 (config in place): the praise prompt.

```
META_API_KEY="$(cat <KEYFILE>)" NET=on KEYFILE=on <T>/bin/bw.sh <T>/bin/a10-t2.sh a10b      # a10-t2.sh:
  <T>/bin/mu-inner.sh exec --provider meta --model muse-spark-1.3-contributor --contributor-mode off --reasoning-effort low \
    --disable-web-tools --disable-reminders --trust-workspace --disable-approval --approval-judge off --max-model-steps 3 "login works now, thanks"
  then poll <T>/store/muse/suggest/ws/*/result.json every 1 s (the writer dies with the sandbox, so the wait is inside it)
```
- **Run 1** (before the state fix): Muse's UserPromptSubmit hook started 22:03:34.662Z (Muse log), `result.json` was written 22:03:49.433Z (file time) = **14.77 s** hook start to result file; from the script's send to the poll that saw the file it was 15.84 s. Outcome proven; promise `#1 Login works now · shadow`; proof 5 of 5 on the start of turn 2, failed 2 of 2 on the start of turn 1; ledger row model `muse-spark-1.3`, 3295 tokens in, 691 out, $0.007056, file mode 600.
- **Re-run on B1** (ws a10b): the script sent the prompt at 23:01:37.333Z and the 1 s poll saw `result.json` 14.48 s later; by Muse's log and the file time the hook started 23:01:38.273Z and the file was written 23:01:51.226Z = **12.95 s** hook start to result file (its `ms` field says 12 946). `result.json`: outcome `proven`, promise 1, saved, 1 call, model `muse-spark-1.3`, 12 946 ms, $0.007033, message "Promise #1 "Login works with session cookie" saved. Proof: passed 5 of 5 on the version from the start of turn 2 and failed 2 of 2 on the start of turn 1. In shadow for now ...". `promise show 1`: step 1 "http: proves login works for demo account alice", step 2 "http: proves login session persists across requests", "Broken copies: the check failed on 1 of 1 copies", "Shadow: 0 of 3 clean days". Writer ledger `<T>/store/muse/suggest/ledger.jsonl` (file mode 600, folder 700): 23:01:38Z, `muse-spark-1.3`, input 3287, cached 0, output 688, $0.007033, status completed, "attempt 1, 3.6s". The writer's state is now under the Torpor home (defect F1). Engine log: turn 1 "edits: auth.js", turn 2 "login works now, thanks". Doctor afterwards (exit 0): NOT VERIFIED, no doctor output of this run was kept. Only the sandbox's own processes were left afterwards.

## Defects found and fixed (each with a regression test)

| Id | Found by | Symptom | Fix | Test |
|---|---|---|---|---|
| ETXTBSY | a flaky `tests/shim_doctor.rs` (failed once on a fresh target dir, per the commit message; that the lead asked for a fix in the production code is NOT VERIFIED, no retained record says so) | a script written a moment ago can still be open for writing in another thread's forked child, and then starting it fails with "Text file busy". Reproduced on the unfixed build: of 40 repeated test runs (54 tests each), run 23 failed `a_shell_start_up_file_that_prints_is_found` with "Text file busy (os error 26)"; after the fix 8 of 8 runs passed | `src/liveness.rs:21 spawn_retry`, used at `src/engine.rs:181`, `src/doctor.rs:1072`, `src/suggest/attach.rs:121`, `src/suggest/mod.rs:526` (commit 54f7b88) | `src/engine.rs:325 a_program_still_open_for_writing_is_started_once_it_is_closed` |
| F1 | A10 run 1 | the writer's state folder followed `XDG_STATE_HOME`, which Muse's hook environment does not have, so a hook-started writer and a typed `torpor-muse suggest` used different folders (two ledgers, two day caps, two locks) | `src/suggest/state.rs:20 state_dir()` now `<TORPOR_HOME>/muse/suggest/` (commit 1b01d36); live: A10 re-run | `the_state_folder_ignores_xdg_so_a_hook_and_a_typed_run_share_it` in `src/suggest/state.rs` |
| D1 | A8 by hand | after the poisoned command was put back, doctor kept printing the old failure for as long as that session log was among the six newest it scans (`LOGS_SCANNED = 6`, `src/doctor.rs:40`; a repaired hook is a new key, the failed key never runs again) | `src/doctor.rs`: `recorded_at` :486, `scan_log` :545 (:587), `check_logs` :624, rule at :650: a failure is history once the settings changed after it and the same event has run fine since (commit 4118000); live: `e2e/run.sh` D1 checks | `tests/accept_doctor.rs` (3 tests); the live negative control was the original A8 symptom on the pre-fix binary |
| E1 | A4c | `hook.rs` dropped every event of a model-spawned subagent, so the subagent ran the store-writing `rm` that the main session was refused | `src/hook.rs:326 adopt`, `:339 turn_ended`; `src/hook/state.rs:208-247` (`open_set`, `open_get`, `open_clear`, 12 h limit): tool events of a subagent go to the engine under its parent's open turn (commit a9458a6) | `tests/accept_subagent.rs` (5 tests) |

**E1 live check on B1** (v2.1): a subagent told to `rm <vault>/e1-fix-probe ; touch $TMPDIR/e1-fix-ran` got "tool blocked by hook: Torpor: this file belongs to Torpor's save points; the AI can't change it."; the probe file was still in the vault and the marker absent; the subagent's PreToolUse record was `blocked` (effects `blocked`, `permission_denied`). In turn 2 an allowed `echo made-by-subagent > sub-made.txt` ran and the engine logged "1 file subagent: sub-made.txt" for that turn (`torpor show 9`: "agent sub-made.txt new, +1 -0", "Save points: 18 start · 20 end").

Known limits of the E1 fix (marked `shortcut:` in `src/hook.rs`): the parent is found by folder alone, so two Muse windows in one folder can swap parents; a subagent that outlives its parent's Stop is not seen.

## Not run, or only partly

- A5c (see above); A9's harder variant; an interrupted editing turn for A7.
- A4, A5, A6, A9 and A7-between-turns ran on B0, not re-run on B1. The E1 commit adds only open-turn bookkeeping (a record written on the main UserPromptSubmit and cleared after a turn-ending event) plus the subagent forwarding; the main session's tool path is otherwise unchanged. Judged low risk, not proven. B1 re-runs: A1, A2, A8 (and D1) by `e2e/run.sh`, A3, A10, the E1 live check, A7 mid-run.
- No Muse plugin, MCP or web tool was used; web tools were off in all keyed runs.
- NOT VERIFIED, each where it occurs above: the `cmp` of the B0 uninstall (A1); doctor output after the A3 turn, after the A7 mid-run and after the A10 re-run; doctor exit statuses of the by-hand runs (only A7 between-turns kept one; B1 checks them in `e2e/run.sh`); the `muse session-message send` attempt (A5c); a promise made with `--check` across compaction (A6); the lead's request behind the ETXTBSY fix; that `pkill -f` was never used again; that no helper-payload folder or key text appeared after each single keyed run.

## Re-running the ECHO-only checks

```
TORPOR_BIN=<pinned engine> MUSE=<muse binary> e2e/run.sh      # TORPOR_MUSE=<adapter>, default target/release/torpor-muse
```
Runs A1, A2, A8 and the D1 repair in a fresh temp folder inside `scripts/sandbox/muse-sandbox.sh --net none` (needs bwrap), prints one PASS/FAIL line per check, the doctor lines it relies on, and "e2e: N of M checks passed". Prints `SKIP: <reason>` and exits 0 when there is no Muse install, no engine, no adapter binary, no launcher, or the sandbox does not start. Not part of `scripts/accept.sh` or CI. Result on B1 under the v3 launcher: 18 of 18 at 23:25Z, and 18 of 18 again at 23:27Z, after the last edit of the script. Re-run in S3a-finish on the final tree (src/ and tests/ unchanged since a9458a6, 02:22Z on 2026-10-27): `e2e: 18 of 18 checks passed.` The SKIP paths, also run in S3a-finish, all print their `SKIP:` line and exit 0: no Muse, no engine, no adapter binary, no bwrap ("muse-sandbox: bwrap is not installed"; that path leaves its temp folder behind, as its SKIP line says).
