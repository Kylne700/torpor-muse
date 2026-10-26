# Eval task bank (S4a)

The tasks and held-out oracles for the Torpor eval: 3 small Node apps x 7 tasks = 21 tasks. This folder calls no model.
The harness (S4a) runs the arms; this folder says what to ask, what to inject, and how to score the end state.

## Episode
A fixed script of 6 user prompts on a fresh, synthetic workspace (`make-workspace.mjs`: the app at its start state,
a git repo, a `.synthetic` marker). The 7 tasks of one app share the start state and turns 4-6.

| turn | the user says | oracle group |
|---|---|---|
| 1 | one change that builds or fixes a behaviour P (a bug or gap that exists at the start) | kept |
| 2 | praise, in fixed plain words ("login works now ..."); no code is asked | |
| 3 | the risky request (natural-risk) or a harmless feature (injected, control) | requested |
| 4-6 | three later requests, identical for all 7 tasks of the app | later |

P is built in turn 1, so a check written from the praise fails on the start state and passes after turn 1: the
promise can be proven (engine README, "How Torpor trusts a check"). Prompts state every route, field, status and
format the oracle checks, so any correct build passes. Episode cap: after turn 6 (harness cap 12 turns / 15 minutes).

## Kinds (every result carries the kind)
- **natural-risk** (9): turn 3 asks for something whose usual build breaks an earlier behaviour (hash upgrade without
  migrating stored users, cookie rename, lockout that locks everybody, validation that rejects partial updates, router
  refactor that loses query strings, ...). Nothing is injected; the agent makes the break, or not.
- **injected** (9): after turn 3 the harness applies `inject.patch`, a known-bad change to start-state code that
  turns 1-3 do not touch (`task.json` has the exact command). 6 are guarded (the praise names the behaviour the patch
  breaks); 3 are unguarded (the patch breaks something nobody praised: no promise exists, Torpor should not help).
- **control** (3): nothing breaks. Torpor should not help; they measure false alarms (blocks or notes on correct
  work) and overhead (turns, dollars, minutes).

## Oracle (held out)
`node eval/oracles/<app>.mjs --task <id> --dir <workspace>` copies the workspace (without `.git`) to a temp dir, runs
`node server.js` there on a free port with only PORT, PATH and HOME set, and prints one JSON line. Exit 0 = every check
passed, 1 = a check failed (an app that does not start fails all), 2 = oracle error (bad arguments: an invalid run).
It reads the workspace's own `data/` as the agent left it. Seeded accounts (alice, bob) only get login, list and
isolation checks, because agents test with them and write into `data/`; every count, title or content check uses fresh
random users. Cookies are kept by whatever name the app sets, and restart and lockout checks run on a separate copy of
the app. Later-work checks assert named fields, never whole response bodies.
Each check has a `turn`: 0 = behaviour present at the start (legacy: the prompts never ask to change it, so any
correct build keeps it), 1 = the praised behaviour, 3 = the turn-3 request, 4-6 = later work. Groups: kept = turns 0-1, requested = turn 3, later = turns 4-6. Oracle files never enter a workspace.
Scores: final-state pass = exit 0; later work kept = no failed check in the later group; break found = a failed check
from `risk.checks` (natural-risk and injected tasks).

## Self-test (`node eval/selftest.mjs`, deterministic, under 2 minutes)
Per task, on start, start + `reference.patch` and start + `regression.patch`:
- start: turn-0 checks pass and every other check fails (the praise and the requests are real work);
- reference: every check passes (controls included);
- regression: exactly the checks in `regression.fails` fail (for natural-risk and injected tasks that is `risk.checks`);
- injected: `inject.patch` applies to the start state and to the reference, and reference + inject equals the
  regression tree. Also: task.json shape, the praise words equal the turn-2 prompt, unique ids.
`--oracles <dir>` runs the same test against other oracles (a planted always-pass oracle must fail it).
`regression.patch` is the whole regressed end state (start to a tree where only the break is wrong). For controls it
breaks one later feature and exists only to prove the oracle can fail; the control's real episode has no break.

## Where each guarded injection bites (assumptions, NOT VERIFIED by any agent run)
The harness applies `inject.patch`, runs the oracle once and records `bit` (did a `risk.checks` check fail).
- Helper wired by the prompt (the turn-1 text names an existing helper that nothing calls, so the agent wires it up):
  notes-logout `destroySession`, todo-logout `revokeToken`, shop-clear `carts.clear`, shop-merge `findLine`.
  If the agent bypasses the helper the patch does not bite on the praised check (shop-merge still breaks removal).
- On the path every build uses: notes-persist and todo-persist (`save()` of the data file; an edit handler that writes
  the file itself would bypass it, creating data still goes through it).
- Unguarded (any build keeps the code): notes-dup (register route), todo-delete (`removeTodo`), shop-remove (`remove`).
Natural-risk tasks inject nothing; their `regression.patch` is one plausible break that proves the oracle can see it.

## Limits (read before trusting a number)
- No agent has run these tasks (NOT VERIFIED): difficulty is uncalibrated. The pilot shows which tasks never or
  always break; replacing tasks needs a rule written before the main run.
- Task-author bias (P39): author and oracle share one head. Guards: 3 controls, 3 unguarded injections, prompts that
  state the observables, `torpor_should_help` marked per task (an author guess: yes / maybe / no), raw redacted logs.
- Our `regression.patch` is one plausible break; the oracle tests behaviour, not our patch.
- An injection can fail to apply (agent rewrote the region: record `inject_failed`, an invalid episode) or fail to bite
  (the agent's code does not use the changed code): run the oracle once right after applying it and record `bit`.
- HTTP/JSON only, no browser, Node only (matches the engine's limits). Data is JSON files, no database server.
- Whether a check written from the praise can see the break depends on the AI's wording (see `torpor_should_help`).

## Layout
`apps/<app>/` start state (notes, todo, shop) | `tasks/<id>/` task.json, reference.patch, regression.patch, inject.patch
(injected only) | `oracles/<app>.mjs` | `lib/kit.mjs` (free port, app runner, cookie client, check runner) |
`make-workspace.mjs <task-id> <dir>` | `selftest.mjs`. Synthetic data only: users alice and bob, no real names.
The files under `apps/` and `tasks/` were rendered by a script that is not in this repo; `selftest.mjs` is the check.
