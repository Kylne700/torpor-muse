# First-question eval: results

Result: the recall bar and the false-claim bar are met. The cost bar is narrowly NOT met: 0.00196 USD per non-claim
prompt with the first question against half of today's 0.00384, which is 0.00192 (ratio 0.510). One non-claim prompt
(N13) was called a claim, so it paid for two requests. Per the brief the code was not changed after the counted run.

Model `muse-spark-1.3`, reasoning effort low. Code under test: `390af2c` (the feature is `491747f`; the commit on top adds
only `eval/first-pass`). The instructions were not touched after the dry run. Dry run 2026-10-28 13:28:58 to 13:29:09
UTC; counted runs 13:29:40 to 13:32:11 UTC (first-question leg, then baseline leg). All costs are the binary's own USD (`lo`: fresh input 1.25, cached input 0.15, output 4.25 per 1M tokens; output
includes reasoning).

How: `run.mjs` runs the real `torpor-muse suggest "<prompt>"` in a fresh copy of `demo/app` (with a `.synthetic`
marker, a scripted engine that always proves the check, a fresh state dir) against a loopback stand-in for the API.
Leg first: the first question goes to the real API, today's request is answered with a free canned `cannot_check`.
Leg baseline: the first question is answered with a free canned `claim` and today's request goes to the real API, so
it is today's single call with today's excerpt (mean input 3,116 tokens, 1,023 cached; the calibration ledger had about
3,170 and 1,140). Every request the binary made was either forwarded (65 in all, one ledger row each) or canned.

## The bars

| bar (from the brief) | result | |
|---|---|---|
| first-question recall on the 20 claims: at least 19/20 | 20/20 (3 Hebrew and 2 typo claims included) | met |
| others called claims: at most 3/20 | 1/20 (N13) | met |
| mean USD per non-claim prompt: at most half of the baseline's | 0.00196 against 0.00192 (ratio 0.510) | NOT met, by 0.00004 |

Accounting rule for the cost bar (implemented in `run.mjs report`, committed before the first keyed call): the cost of a
non-claim prompt is the first question's USD plus, only when the first question did not say `not_a_claim`, the cost of
today's call for the same prompt (its measured baseline cost when the prompt is one of the ten baseline others, else the
mean of those ten, 0.00384). Only N13 is such a prompt and it is not a baseline prompt, so it adds an estimate (0.00384,
not measured) and 0.00019 to the mean. Other readings, for the lead to weigh:

- The first question's own cost on the 20 others (N13's 0.00438 included, nothing added for its follow-up call): 0.00176,
  ratio 0.460, bar met. (On a non-claim prompt the first question is the whole cost; N13 is the only case where it is not.)
- The 10 others measured both ways: first question 0.00156 against today's 0.00384, ratio 0.407, bar met.
- Without N13 the 19 others the first question got right cost 0.00163 on average.

## The one miss

- N13, "I wonder whether login will keep working once we add a database": the first question answered `claim` (reason
  empty). It is a speculation, not a claim. It also thought longest of the 40 calls: 989 reasoning tokens, 0.00438 USD,
  so the one false claim cost more than twice an average first question. The rules say "if you are unsure, answer claim",
  so this is the intended direction of an error: such a prompt goes on to today's call and nothing is lost.
- No claim was missed. No first question failed (all 65 forwarded requests were HTTP 200 with an answer that was
  usable).

## Cost per group (USD, lo)

| | claims | others |
|---|---|---|
| first question, mean over the 20 | 0.00123 | 0.00176 |
| today's single call, mean over the 10 measured | 0.00720 | 0.00384 |
| with the first question, mean over the 20 | 0.00843 (first question + today's call) | 0.00196 (see the rule above) |
| change | +0.00123 (+17%) | -0.00188 (-49%) |

Tokens (means over all calls of the counted runs): first question 292 input of which 241 cached, 328 output of which 306
reasoning; today's call 3,116 input of which 1,023 cached, 647 output of which 527 reasoning. The first question's cost
is almost all reasoning. Median seconds: first question 1.50, today's call 2.64, so a claim now waits about 1.5 s more
before today's call starts (the writer runs in the background after the prompt).

Derived from the table: the new flow is cheaper overall when fewer than about 60% of the prompts are claims
(0.00188 saved on a non-claim against 0.00123 added on a claim: 0.00188 / (0.00188 + 0.00123) = 0.60).

## Dry run (5 prompts that are not in the set; `dry.md`, `dry-first.jsonl`)

xC01 and xC02 (claims) came back `claim`; xN01, xN02 and xN03 (a wish, a question, pasted code) came back
`not_a_claim`; 5/5. USD per prompt 0.0014 to 0.0019. No instruction fix was needed, so the instructions of the counted run
are the ones of the commit.

## Spend

`scratch/torpor-s3/spend-W2.md` (kept outside the repo): 65 calls, USD lo 0.1778, hi 0.3200 of the 1.50 cap (dry run lo
0.0076, first leg 0.0598, baseline leg 0.1104). `hi` counts the cached tokens twice and reasoning again, so lo is the
better estimate.

## What this does not show

- 20 prompts per group, one run per prompt, one small single-file app; the baseline is 10 per group. One prompt moves the
  cost ratio across the bar, so the ratio 0.51 is not distinguishable from 0.50 by this run.
- Real prompts from real sessions were not used (the set is written for this test), and the hook path (a prompt in
  Muse Code) was not used: the CLI `suggest` runs the same code after the trigger.
- Which prompts really are claims in daily use decides the saving; the break-even share above is the number to compare
  with.

Reproduce the tables below with `node eval/first-pass/run.mjs report eval/first-pass/counted-first.jsonl eval/first-pass/counted-baseline.jsonl`.
The raw rows (one line per prompt: the verdict, tokens, seconds, USD, and the first line of the binary's output) are
`counted-first.jsonl`, `counted-baseline.jsonl` and `dry-first.jsonl`. One difference to the text above: the generated
report's "median" is the upper one of the two middle values when the count is even (today's call 2.73; the true median,
used in this text, is 2.64); no bar depends on it.

## Generated report

Model muse-spark-1.3; code 390af2cf64835c100df8771f1e25ebcd922152b1; first-question run 2026-10-28T13:29:40.331Z to 2026-10-28T13:30:48.234Z.

| | claims | others |
|---|---|---|
| prompts | 20 | 20 |
| first question said "claim" | 20 | 1 |
| first question said "not_a_claim" | 0 | 19 |
| no usable answer (counts as claim) | 0 | 0 |
| mean USD of the first question (lo) | 0.0012 | 0.0018 |
| median seconds of the first question | 1.48 | 1.54 |

Recall on claims: 20/20 (bar: at least 19/20). Others called claims: 1/20 (bar: at most 3/20).

Misses (1):
- N13 other called claim: "I wonder whether login will keep working once we add a database" -> claim

Baseline: today's single call on 10 claims and 10 others (run 2026-10-28T13:31:12.828Z, code 390af2cf64835c100df8771f1e25ebcd922152b1).

| USD per prompt (lo) | today (baseline) | with the first question |
|---|---|---|
| others, mean | 0.0038 (10 measured) | 0.0020 (all 20; 1 went on to today's call) |
| claims, mean | 0.0072 (10 measured) | 0.0084 (all 20) |
| others measured both ways, mean | 0.0038 | 0.0016 |
| claims measured both ways, mean | 0.0072 | 0.0084 |

Cost bar: mean USD per other prompt with the first question 0.0020 against half of today's 0.0019: NOT met (ratio 0.51).
Median seconds of today's call: 2.73.

Today's call on the baseline prompts:
- C01 check $0.0073
- C03 check $0.0079
- C05 check $0.0078
- C07 check $0.0059
- C09 check $0.0056
- C11 check $0.0080
- C13 check $0.0073
- C16 check $0.0065
- C18 check $0.0092
- C20 cannot_check $0.0065
- N01 not_a_claim $0.0039
- N03 not_a_claim $0.0039
- N04 not_a_claim $0.0036
- N06 not_a_claim $0.0039
- N07 not_a_claim $0.0038
- N09 not_a_claim $0.0044
- N12 not_a_claim $0.0038
- N14 not_a_claim $0.0035
- N17 not_a_claim $0.0040
- N20 not_a_claim $0.0036

| id | first question | USD lo | input/cached/output/reasoning | seconds | prompt |
|---|---|---|---|---|---|
| C01 | claim | 0.0010 | 287/241/222/205 | 1.37 | "login works now" |
| C02 | claim | 0.0012 | 289/241/253/236 | 1.48 | "Great, signing in works and the account page shows the righ |
| C03 | claim | 0.0013 | 292/241/288/271 | 1.93 | "I just tried it by hand and logging in with the demo user w |
| C04 | claim | 0.0014 | 288/241/316/299 | 1.49 | "the login is fixed, it returns the user now" |
| C05 | claim | 0.0009 | 293/241/177/160 | 1.09 | "Sign-in works. Next, please add a logout button to the acco |
| C06 | claim | 0.0012 | 295/241/251/234 | 1.47 | "login works now and /api/me answers 200 after signing in" |
| C07 | claim | 0.0011 | 288/241/229/212 | 1.48 | "yes! the session cookie is set and the account page loads" |
| C08 | claim | 0.0012 | 296/241/265/248 | 1.84 | "Everything works: login, then the profile call, no errors." |
| C09 | claim | 0.0011 | 293/241/230/213 | 1.42 | "wrong passwords are rejected with a 401 now, that part work |
| C10 | claim | 0.0013 | 291/241/279/262 | 1.01 | "perfect, it works - I can log in and see my name" |
| C11 | claim | 0.0013 | 297/241/274/257 | 1.34 | "that fixed it, login is working again. can you now update t |
| C12 | claim | 0.0013 | 286/241/274/257 | 1.36 | "the health check answers ok now" |
| C13 | claim | 0.0014 | 282/241/317/300 | 1.68 | "logn works now" |
| C14 | claim | 0.0012 | 294/241/251/234 | 0.97 | "the acount page works, it shows my name after loging in" |
| C15 | claim | 0.0013 | 297/241/287/270 | 1.78 | "Tested it by hand and signing in works, so let's move on to |
| C16 | claim | 0.0011 | 288/241/232/215 | 2.07 | "ההתחברות עובדת עכשיו" |
| C17 | claim | 0.0010 | 301/241/218/201 | 1.54 | "הכניסה עם משתמש הדמו עובדת, והעמוד האישי מציג את השם שלי." |
| C18 | claim | 0.0018 | 305/241/403/386 | 1.98 | "מעולה, זה עובד! אפשר להתחבר ולראות את החשבון. עכשיו תוסיף כ |
| C19 | claim | 0.0014 | 294/241/317/300 | 1.65 | "Looks like login is working now; the old 500 error is gone. |
| C20 | claim | 0.0010 | 299/241/219/202 | 1.28 | "logging out works too: after it, the account page says you  |
| N01 | not_a_claim | 0.0015 | 283/241/326/296 | 2.57 | "does login work?" |
| N02 | not_a_claim | 0.0018 | 285/241/401/374 | 1.79 | "make login work" |
| N03 | not_a_claim | 0.0016 | 283/241/362/337 | 1.44 | "login doesn't work" |
| N04 | not_a_claim | 0.0016 | 281/241/358/331 | 1.54 | "continue" |
| N05 | not_a_claim | 0.0012 | 281/241/272/240 | 1.6 | "yes do it" |
| N06 | not_a_claim | 0.0011 | 291/241/232/204 | 1.19 | "please add a logout button to the account page" |
| N07 | not_a_claim | 0.0017 | 324/241/356/326 | 1.49 | "TypeError: Cannot read properties of undefined (reading 'na |
| N08 | not_a_claim | 0.0017 | 295/241/379/349 | 1.57 | "why does the login answer 500 when I send an empty body?" |
| N09 | not_a_claim | 0.0018 | 290/241/408/379 | 1.73 | "can you check whether the session cookie is HttpOnly?" |
| N10 | not_a_claim | 0.0016 | 295/241/356/326 | 1.5 | "Error: listen EADDRINUSE: address already in use :::3000" |
| N11 | not_a_claim | 0.0019 | 298/241/413/384 | 1.65 | "next I want a page that lists all users, let's plan it firs |
| N12 | not_a_claim | 0.0016 | 288/241/356/327 | 1.47 | "ok thanks, that is all for today" |
| N13 | claim | 0.0044 | 294/241/1006/989 | 4.4 | "I wonder whether login will keep working once we add a data |
| N14 | not_a_claim | 0.0015 | 292/241/339/309 | 1.2 | "login still fails with the demo user, it says wrong passwor |
| N15 | not_a_claim | 0.0017 | 288/241/374/344 | 1.44 | "the account page should show the user's name after login" |
| N16 | not_a_claim | 0.0016 | 291/241/360/333 | 1.52 | "run the tests and tell me which ones fail" |
| N17 | not_a_claim | 0.0014 | 291/241/306/278 | 1.18 | "rename the variable tok to token in server.js" |
| N18 | not_a_claim | 0.0018 | 298/241/404/377 | 1.9 | "what does the /api/me route return when I am not signed in? |
| N19 | not_a_claim | 0.0019 | 291/241/427/398 | 2.07 | "undo that, it broke the login" |
| N20 | not_a_claim | 0.0018 | 288/241/403/377 | 1.4 | "האם ההתחברות עובדת?" |
