# Eval: the pre-registration (draft for v1)

Written on 2026-10-26, before any run. It fixes how the eval in `eval/DESIGN.md` is run, scored, analysed and published, so that nobody can say the rules were bent after the results were in. The values in angle brackets are filled in by the lead after the pilot. The lead then reads this file, `eval/run/RUN.md` and `eval/analysis/analyze.mjs` and, if they are good, tags the commit `eval-prereg-v1`. After the tag, these files change only through the amendment rule in section 11.

**Every result is published, the ones we hoped for and the ones we did not.** The three comparisons of section 7.2 are published whatever they say, and every task where arm D does not beat arm C stays visible in the per-task tables.

## 1. Questions and hypotheses

- H1, the primary question, "what did the new work for Muse Code add to the engine": arm D passes the held-out oracle at the end of the episode more often than arm C, on the same (task, repetition) pairs. Smallest effect of interest: 0.4 absolute in pass rate (for example 0.8 against 0.4).
- H2, secondary, labelled "engine + new work": arm D passes more often than arm A.
- H3, secondary: arm B, a plain notes file, passes more often than arm A. It shows how much of an effect a notes file gets without Torpor.
- The null for all three is no difference in pass rate. The tests are two-sided although the hypotheses have a direction; a result in the other direction is reported as it is (section 7.2). Only H1 gets a verdict word; H2 and H3 are reported as numbers.
- If arm C scores about the same as arm A (for example because the engine's hooks fail open on Muse Code), the report says so plainly.

## 2. Arms

| Arm | What it is |
|---|---|
| A | Nothing. Muse Code as it comes: no hooks, no instruction file. |
| B | A notes file: an `AGENTS.md` in the workspace that tells the agent to keep a `NOTES.md` of the behaviours the user confirmed working and to re-check them before it says it is done. It never mentions Torpor. |
| C | The engine hand-wired, no adapter: the engine's documented `torpor hook <Event>` commands put into Muse Code's settings with the event names mapped, each run as `env TORPOR_HOME=<episode>/torpor-home TORPOR_KEY_FILE=<episode home>/.config/torpor/key <engine> hook <Event>`, nothing else. The same wiring as arm 3 of the disaster drill (`drill/PREREG.md`). |
| D | The adapter plus the engine: `torpor-muse init` into the episode's own home folder, built from the adapter commit that every row records. |

The exact text of the notes file, the exact settings written for C and D, the flags of every Muse Code call, the sandbox, and what `usd_hi` contains are in `eval/run/RUN.md` (written by the runner stage; the lead checks that it matches this section before tagging) and, for each episode, in the `manifest` of its row. Arms C and D use the engine pinned in `engine.lock` (tag `pre-event-final`). Whether the adapter's writer (the model call that proposes checks) is on in arm D, and under which key and cap per episode (a fresh ledger for each; the number and how it was derived are in `eval/run/RUN.md`), is fixed there; the rows record its calls and dollars apart (`writer_calls`, `writer_usd_hi`) and what it decided (`writer_results`: turn, time, outcome, whether a promise was saved, dollars, and a note of at most 160 characters).

With the writer on, the launcher waits for the detached writer after every turn, up to 60 s more (`manifest.writer_drain_s`). That wait is not model time: the turn limit is extended by it and the episode cap does not count it. A writer (or any other leftover process) still running when the wait ends is stopped and recorded as `still_running`; the episode stays valid, and what that writer would have written is not counted.

The arms differ in these installs only. Everything else is the same: the Muse Code build, the model id and provider, the six scripted prompts, the workspace builder, the sandbox and the caps. Every row records the Muse Code version, the engine version and the adapter commit it ran with.

## 3. Tasks

21 tasks, `eval/tasks/` at the tagged commit: 3 small Node apps x 7 tasks. An episode is 6 scripted user prompts on a fresh synthetic workspace; the oracle runs after the last one (`eval/DESIGN.md`).

| App | natural-risk (9) | injected, guarded (6) | injected, unguarded (3) | control (3) |
|---|---|---|---|---|
| notes | cookie, hash, ratelimit | logout, persist | dup | ctrl |
| todo | router, tags, validate | logout, persist | delete | ctrl |
| shop | coupon, customer, vat | clear, merge | remove | ctrl |

- Natural-risk: the request in turn 3 is one whose usual build breaks an earlier behaviour; nothing is injected. Injected: after turn 3 the harness applies a known-bad patch; a guarded one breaks a behaviour the user praised, an unguarded one breaks something nobody praised, so no promise exists. Control: nothing breaks; the controls measure false alarms and overhead.
- Each task carries `torpor_should_help` (yes, maybe or no), an author's guess made before any run: 13 yes, 1 maybe (todo-tags), 7 no (notes-ratelimit, the 3 unguarded injections and the 3 controls). Tasks where Torpor is not expected to help are in the bank on purpose, so that a win which shows only where the author expected one can be seen.
- The held-out oracle (`eval/oracles/`) decides pass or fail. It, the task files and the patches never enter a workspace that a model runs in.
- No task is replaced or changed because of how the arms scored on it. A task may be fixed or replaced before the tag only for a defect in the task bank (an oracle error, an injection that cannot be applied to the reference tree), and each such change is an amendment (section 11). Tasks that every arm passes, or none does, stay in; the per-task tables show them.

## 4. Sample size, pairing and randomisation

- n per arm (planned episodes before exclusions): A, C and D: `<N from the pilot>`; B: `<N_B from the pilot>`. n is fixed at the tag and never topped up. The planning figures were 40 for A, C and D and 20 for B; with 21 tasks that is 2 repetitions per task (42 episodes) and 1 repetition (21 episodes). The lead decides after the pilot.
- Power: for two independent groups of 40, an exact test (Fisher, two-sided, alpha 0.05) has power 0.947 for 0.8 against 0.4 and 0.725 for 0.7 against 0.4 (exact enumeration, computed for this file). This is a rough guide only: the analysis here is paired, the repetitions of one task are not independent, and the verdict rule needs the whole cluster-bootstrap interval above 0, so the real power is lower than these figures. The power of the full rule was not simulated.
- A pilot (planned: 12 episodes, 3 tasks x 4 arms) is run before the tag to measure dollars, minutes, the rate-limit rate and the share of valid runs. It is not a scored run, it may inform n, and its rows are not part of the main analysis (draft default).
- Pairing: every (task, repetition) is played in every arm of the main run. A pair is the same task and the same repetition number in two arms. Arm B has fewer repetitions; its pairs with arm A are the repetitions the two share.
- Randomisation: the order of the arms inside every (task, repetition) group is drawn from a seeded generator. The run seed is `<run seed>`; every row records `seed` and `order_index`. The runner plays one group at a time, all arms of it, so that a forced stop leaves complete pairs (`eval/run/RUN.md` must say so; the lead checks). Every episode has its own fresh workspace, home folder and Torpor store; nothing carries over from one episode to the next.

## 5. Outcomes

- Primary (H1): `final_pass`, arm D against arm C. `final_pass` is true when the held-out oracle exits 0 on the workspace after the last turn, which means every check passed. An episode that hits the 15-minute cap counts as a fail in every arm.
- Secondary, published for every arm and every kind, all descriptive:
  - `later_kept`: no failed check in the later-work group (turns 4 to 6); a timeout also counts as a fail here.
  - `break_found` (natural-risk and injected tasks only): a check from the task's `risk.checks` failed. The paired rows show its mirror, "no break found".
  - turns, minutes, steps and dollars per episode (`usd_hi`; the writer's dollars shown apart), gate blocks per episode, timeouts, `rate_limited` and `hook_failures`.
  - false alarms: control episodes with at least one gate block.
  - per arm, the units excluded and their reasons.
- No outcome is added after the tag.

## 6. Validity, exclusions and re-runs

- An infra failure has nothing to do with the arm: the sandbox could not start, Muse Code crashed with no hook involved, the rate-limit retries ran out, the disk or memory ran out, or the oracle exited 2. The row is written with `valid` false and an `invalid_reason`, and the episode is re-run in a fresh workspace, at most 2 times (3 attempts in all). Every attempt is counted and published.
- A failure of Torpor itself is not an infra failure: a hook that exits non-zero, times out or panics, an engine that cannot read Muse Code's payload, an adapter that gets a tool name wrong. These are results: the row stays valid, `hook_failures` counts them, and the oracle decides pass or fail.
- A timeout is a result, not an infra failure: a fail in every arm.
- Network lines other than an allowed `api.meta.ai:443` in an episode's proxy log (`egress.other` in the row) are recorded and reported per arm. They do not make an episode invalid: the proxy refuses those connections, so they cannot change the outcome.
- An injection that could not be applied (the arm's agent rewrote the region in turns 1 to 3: `inject_failed`) is invalid, not re-run, counted and listed with its reason. Whether the region was rewritten depends on the arm's own work, so a re-run would pick episodes by what the arm did. The unit is excluded (section 7.1).
- A held-out file found in the episode folder after a turn (`held_out_violation`: one of the oracle's, the task's or the patches' files, by name or by content hash) is invalid, not re-run, counted and listed with its reason. The promise that the model never sees held-out files failed in that episode, and a re-run would hide that instead of reporting it. The unit is excluded (section 7.1).
- An injection that was applied but did not bite (the agent's code does not use the changed code) stays valid (draft default). Whether it bites depends on the arm's own work, so dropping those episodes would condition on something the arm influenced.
- If the third attempt fails too, or the only attempt is one that is not re-run (`inject_failed`, `held_out_violation`), the unit is excluded and stays missing; nothing replaces it. The report names these units "excluded, no valid attempt (3 failed attempts, or one not re-run)". Excluded units are counted per arm, with their reasons, next to the result.
- No other episode is excluded or repeated. A bad-looking result is not a reason for a re-run.
- A pair needs both units valid. A valid unit without a partner counts in the per-arm tables and in no paired test.

## 7. Analysis

The analysis is `eval/analysis/analyze.mjs` at the tagged commit (no dependencies, Node 20 or later). `node eval/analysis/selftest.mjs` checks it on synthetic rows with known answers. It is run once on the main-run rows:

`node eval/analysis/analyze.mjs --rows <rows folder> --md <out.md> --json <out.json> --seed 1 --boot 10000`

Its markdown and JSON are published as written. Every output names the sha256 of the script, the model ids, the run ids and the seed.

### 7.1 Reading rows

- One JSON file per attempt, schema 1. A row with another schema, a missing key, or a value of the wrong type or outside its set makes the script refuse the whole folder, with one line per problem; nothing is analysed. A JSON file that holds none of `arm`, `task`, `rep`, `attempt`, `valid` and `final_pass` is not a row: it is skipped and listed.
- A unit is (arm, task, repetition). Its row is its first valid attempt (lowest attempt number); later attempts of a unit that has a valid one are ignored and counted. A unit with 3 invalid attempts, or with an invalid attempt the runner marked as not to be re-run (`retryable` false, section 6), is excluded ("exhausted", reported as above). A unit with fewer attempts and none valid is excluded and flagged as pending (an unfinished run). The script refuses more than 3 attempts for a unit, a duplicate attempt, and a task that appears with two kinds.
- One model id per arm and per comparison. Mixed ids in a comparison are refused, so each model gets its own folder and its own analysis.
- A timeout makes `final_pass` and `later_kept` false whatever the oracle says; `break_found` stays as recorded. The script warns when it changed a row this way.

### 7.2 The three pre-registered comparisons

D against C (primary), D against A ("engine + new work") and B against A, each on `final_pass`, all kinds pooled, on pairs.

- b = pairs where the first-named arm passed and the second failed; c = the reverse. Effect = (b - c) / pairs, the difference in pass rate.
- p: exact two-sided McNemar, p = min(1, 2 x P(X <= min(b, c))) with X binomial(b + c, 1/2). With no discordant pairs p = 1.
- Interval: 95 % cluster bootstrap. It resamples tasks with replacement (all pairs of a drawn task come along), recomputes the effect on each resample, and takes the 2.5 % and 97.5 % percentiles of 10000 resamples. The generator is seeded with 1, so the same rows give the same interval. Other seeds may be shown beside it to show that the interval is stable; the headline is seed 1. With fewer than 2 tasks there is no interval, and the verdict is "inconclusive at this n".
- Verdict words, fixed now, no others: "D better than C" only if p < 0.05 AND the whole interval is above 0 (strictly); "inconclusive at this n" otherwise, always with the effect and the interval. Only D against C gets a verdict word. D against A and B against A are secondary: the script prints their b, c, p, effect and interval and no verdict word. A result in the other direction (the interval entirely below 0) is also "inconclusive at this n"; the line then adds a plain note that the interval lies below 0.
- Why both conditions: the McNemar p treats pairs as independent, and repetitions of one task are not; the interval resamples tasks and so carries that. Both must agree. No correction for several comparisons is applied; only D against C is the primary.

### 7.3 Everything else

- Every proportion is k/n with a 95 % Wilson interval (z = 1.959964). Zero events in n print "under about 3/n" (the rule of three: 0 in 40 means under about 8 %).
- Per arm and per kind: medians and totals of minutes, turns, steps and dollars, gate blocks per episode, rate-limit and hook-failure counts, exclusions by reason.
- Paired rows for the secondary outcomes and for each kind are printed with p and interval but no verdict word and no correction. They are descriptive.
- Per-task k/n tables for each of the three comparisons, with the task's kind and `torpor_should_help`.
- Dollars are `usd_hi` as recorded. The writer's dollars are shown apart and never added to it.

## 8. No peeking, no optional stopping

- n is fixed at the tag. The main run is analysed once, after the last planned unit has finished or is excluded.
- Until then only operational numbers are looked at: units finished, spend, minutes, the rate-limit and invalid shares. No pass or fail count of the main run, per arm or per task, is looked at, and `analyze.mjs` is run on pilot rows and synthetic rows only.
- If the run must stop early (money, time, rate limits), the analysis uses the units that finished, says so at the top with the reason, and no unit is added, dropped or re-run after anyone has seen a result. The verdict rule is unchanged; a smaller n shows up as a wider interval.

## 9. The standard-model confirmation subset

The main run uses the contributor id `muse-spark-1.3-contributor`; whether it has the same weights as the shipped model is NV. To see whether the direction holds on the standard id, a subset runs on `muse-spark-1.3`: arms A and D only, `<subset episodes per arm>` episodes per arm, on the tasks `<subset tasks>`, at most $40 in total (enforced by the runner's spend cap). The lead fixes the tasks and the size from the pilot's measured cost before the tag.

- It is directional only, with no significance claim. It goes through the same script into its own folder (one model id per folder). What is quoted is the pass counts of A and D, the effect and its interval, labelled "standard model, directional"; the script prints no verdict word for it (D against C is not in that folder, and D against A never has one).
- It is never pooled with the main run. If it points the other way, the report says so next to the main result.
- Its pass counts are not looked at before the main analysis is done.
- If the cap stops it early, the finished episodes are reported as they are.

## 10. What the results can and cannot claim

- Can claim: "on Muse Code, with model id X, on N synthetic tasks, the adapter arm kept the app's behaviours in p versus q". X is the id in the tables, N the number of valid pairs, p and q the pass rates of arms D and C.
- Cannot claim: results on real projects; results for the shipped model (the contributor id is not known to be the same); results for human users; results for the engine alone (the engine is in arms C and D alike, so D against C measures what the adapter added).

Limits, stated in advance:

- The tasks, the oracles and the injections were written by one AI agent, so the author and the oracle share one head. Guards: 3 controls, 3 unguarded injections, prompts that state the observables, `torpor_should_help` marked before any run, and the redacted logs.
- No agent had run the tasks when the bank was written, so difficulty is uncalibrated and some tasks may always or never break. Those results stay in.
- The data is synthetic. The apps are HTTP/JSON Node apps with no browser and no database server. Episodes are 6 turns.
- Whether an injection bites depends on how the arm's agent wrote the code.
- In arms C and D, whether a check written from the user's praise can see a break depends on the AI's own wording.
- Repetitions of one task are not independent samples. The interval resamples tasks, and with 21 tasks it is wide.
- One model family, one Muse Code build, one sandbox.

## 11. Publication and amendments

- For every episode the runner stores one row per attempt, valid or not. Nothing is dropped. Published: all rows, the analysis output as written, the list of excluded units with their reasons, and the redacted logs (Meta's terms for publishing contributor logs: NV; if they forbid it, the logs stay unpublished and this is stated).
- Before the first scored main-run episode, this file, `eval/run/RUN.md`, the task bank and `eval/analysis/analyze.mjs` may be amended, the pilot and the angle-bracket values included. Each amendment is a commit that adds a dated entry to the log below with what changed, why, and the words "no scored run has happened". The lead puts the next tag (`eval-prereg-v2`, `eval-prereg-v3`, ...) on the commit of each amended version. A tag is never moved or deleted.
- After the first scored main-run episode, nothing in sections 1 to 8 changes. A defect in the runner, an oracle or the analysis may be fixed, but the fix is logged here with the reason, every affected episode is repeated or the analysis re-run, and both results are published.
- The lead tags the approved version `eval-prereg-v1`.

Values the lead fills in before the tag: `<N from the pilot>`, `<N_B from the pilot>`, `<run seed>`, `<subset episodes per arm>`, `<subset tasks>`.

Amendment log:

- 2026-10-27, draft edits before the tag `eval-prereg-v1` (the lead tags it). No scored run has happened. Source: the lead's plan line "S4a-run-2 DONE" and the unscored writer-on episode s4a-d-writer-1. What changed and why:
  - Section 2: arm C is stated as the runner wires it (`env TORPOR_HOME=...`, `TORPOR_KEY_FILE=...`), because the text left both out. The writer cap is per episode, not daily; the 0.05 the runner used refused every writer trigger after the first (one call's worst case is about $0.043), so the runner default is now 0.35 (derivation in `eval/run/RUN.md`). The writer's drain, the `still_running` rule and the new row field `writer_results` are stated, because RUN.md already had the drain and the rows now record what the writer decided.
  - Section 6: `held_out_violation` is listed as invalid and not re-run, and `egress.other` is ruled to be reported per arm without invalidating an episode; the runner already treated both that way, and the analysis now reports `egress.other` per arm. The exclusion rule and its report label now also cover a unit whose only attempt is not re-run (the analysis labelled all excluded units "after 3 failed attempts").
