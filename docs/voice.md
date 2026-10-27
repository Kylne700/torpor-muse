# Voice promises: `torpor-muse say`

You say out loud what works, and get a check, the same as if you had typed it to `torpor-muse suggest`.

    torpor-muse say <clip.wav> [--keywords | --no-keywords]

1. The clip is checked on your machine (the WAV header, see "The clip").
2. It is sent to Meta's Muse Voice Transcribe API (`POST /v1/asr/transcribe`, model `muse-voice-transcribe-1.0`, push-to-talk mode).
3. What it heard is printed as `heard: "..."`.
4. That text goes to the check-writer exactly as if you had typed it, with the same exit codes: 0 a check was saved, 3 nothing was saved, 1 an error, 2 wrong use. If the text has no letter or digit, `say` prints `advisory: nothing was heard in the clip, so no check was made`, exits 3 and does not call the writer.

The input is a file. There is no microphone code. The words that come back are treated as data: control characters are removed and the text is only ever used as a typed claim.

## Privacy

Privacy: the audio file is sent to Meta's Voice Transcribe API; Torpor stores no audio.

Keywords are on by default (measurement below): up to 24 words from the names of your code files and the URL paths in your code are sent along with the audio, so that the transcription favours your own words. `--no-keywords` sends none. Anything that looks like a key is masked before the words are taken, and the files read are the same ones the check-writer is allowed to read.

## The clip

WAV, PCM, mono, 16-bit, 16000 or 24000 Hz, at most 10 minutes and 32 MB. Anything else gives exit 1, nothing is sent, and the message shows how to convert it:

    ffmpeg -i input -ac 1 -ar 24000 -c:a pcm_s16le -map_metadata -1 output.wav

Only the header is checked. A file with a good header that holds no speech is sent anyway.

## Cost and errors

- USD 0.18 per hour of audio, counted in whole seconds by the audio length the server reports. The call goes through the same ledger and day cap as the check-writer (default USD 2 a day, `TORPOR_MUSE_DAY_CAP_USD`): the cap is checked before the call, a row is written after it. A network error or a 5xx is charged at the worst case, any other error costs nothing.
- The key is found the same way as for `suggest` (`META_API_KEY`, or `key_file` in `~/.config/torpor-muse/config`). It is never printed or logged.
- Status 400, 401, 403, 406, 413, 429, 5xx and network errors each give one line, exit 1, and nothing is saved.

## Keywords

The keywords are the project's own words: the words in the names of its code files and in the URL paths written in its code (for example `/login`), the most repeated first, at most 24.

### Measurement

Question: do the keywords help the transcription? The claims, the project words, the scoring and the rule below were committed in `scripts/voice-measure.mjs` before the first call to the real API.

- Six English claims, taken from the first two different praise lines of each of the three apps in `eval/apps`. Three flite voices (slt, rms, awb) say each one: 18 clips, 24 kHz mono. Each clip was transcribed once without and once with the keywords: 36 calls, one try per transcript, no `languageBias` in either arm.
- The keywords sent are what the shipped derivation gives for the app the claim is about (listed under the table). None was picked by hand.
- A clip is a hit when every project word of its claim is heard as a whole word (lowercased, punctuation removed). An artifact is a doubled word, or a project word stuck to another word ("loginlogin").
- Rule, fixed beforehand: keywords are on by default only if hits with > hits without and artifacts with <= artifacts without. Otherwise they stay off and `--keywords` turns them on.
- A call that failed would have been tried again after 10 s, up to 3 tries, and the tries are in the rows. None failed.

Result: 11 hits with keywords against 6 without, 2 artifacts against 6. The rule holds, so keywords are on by default. By app (6 clips each):

| app | hits without | hits with | artifacts without | artifacts with |
|---|---|---|---|---|
| notes | 0 | 3 | 3 | 0 |
| shop | 3 | 3 | 1 | 0 |
| todo | 3 | 5 | 2 | 2 |

How far to trust it: synthetic voices (flite), one run each, 18 clips, and the service does not always give the same words for the same clip (seen in trials before this run). In the todo app the keyword list does not contain the project words of the claims (todo, done, filter), yet its hits went from 3 to 5. So part of the difference is noise, and nothing here says how real human speech behaves. Only the notes app, whose list holds `login` and `logout`, shows a clear gain.

Every transcript (synthetic voices (flite), one run each). The rows are in `docs/voice-rows.jsonl`; `node scripts/voice-measure.mjs table docs/voice-rows.jsonl` makes this table again.

| clip | claim | voice | without keywords | hit | artifact | with keywords | hit | artifact |
|---|---|---|---|---|---|---|---|---|
| c1-slt | 1 | slt | LogoutLogout works now. The old cookie is dead after I log out.. | no | yes | log out works now the old cookie is dead after i log out. | no | no |
| c1-rms | 1 | rms | log out out works now. The old cookie is dead after I log out.. | no | yes | logout works now, the old cookie is dead after I log out. | yes | no |
| c1-awb | 1 | awb | Look out works now the old cookie is dead after I log out. | no | no | lookout works now. The old cookie is dead after I log out. | no | no |
| c2-slt | 2 | slt | Legan works now for Alice and Bob whatever whatever capitalization I type. | no | yes | login works now for Alice and Bob whatever capitalization I type | yes | no |
| c2-rms | 2 | rms | Legan works now for Alice and Bob, whatever capitalization I type.. | no | no | legan works now for Alice and Bob, whatever capitalization I type. | no | no |
| c2-awb | 2 | awb | Ligon works now for Alice and Bob whatever capitalization I type. | no | no | login works now for Alice and Bob, whatever capitalization I type. | yes | no |
| c3-slt | 3 | slt | check out empty's the cart now, so nothing is ordered twice.. | no | no | check out empties the cart now, so nothing is ordered twice.. | no | no |
| c3-rms | 3 | rms | check out empties the cart now so nothing is ordered twice. | no | no | check out empties the cart now so nothing is ordered twice | no | no |
| c3-awb | 3 | awb | Check out empties the car now, so nothing is ordered twice. | no | no | check out empties the cart now so nothing is ordered twice. | no | no |
| c4-slt | 4 | slt | Adding the same product twice now gives one line with the right quantity and total. | yes | no | adding the same product twice now gives one line with the right quantity and total. | yes | no |
| c4-rms | 4 | rms | Adding the same product twice now gives one line with the right quantity and total. | yes | no | adding the same product twice now gives one line with the right quantity and total. | yes | no |
| c4-awb | 4 | awb | I think the same product twice now gives one line line with the right quantity and total. | yes | yes | i think the same product twice now gives one line with the right quantity and total. | yes | no |
| c5-slt | 5 | slt | Marking a a Toto Don't Works Now, and it sticks after a restart.. | no | yes | marking a todo done works now and it it sticks after a restart. | yes | yes |
| c5-rms | 5 | rms | marking a todo done works now and it sticks after a restart. | yes | no | marking a todo done works now and it sticks after a restart.. | yes | no |
| c5-awb | 5 | awb | marking a total downworks now, and it sticks after the restart. | no | no | marking a total done works now and and it sticks after a restart. | no | yes |
| c6-slt | 6 | slt | The Dunfilter works now for both true and false. | no | no | the done filter works now, for both true and false.. | yes | no |
| c6-rms | 6 | rms | The Done filter works now for both true and true and false.. | yes | no | the done filter works now for both true and false. | yes | no |
| c6-awb | 6 | awb | The done filter filter works now for both true and false.. | yes | yes | the done filter works now, for both true and false. | yes | no |

The claims and the project words that must be heard (a clip is a hit when all of them are):

1. (notes) "Logout works now: the old cookie is dead after I log out." project words: logout, cookie
2. (notes) "Login works now for alice and bob, whatever capitalisation I type." project words: login, alice, bob
3. (shop) "Checkout empties the cart now, so nothing is ordered twice." project words: checkout, cart
4. (shop) "Adding the same product twice now gives one line with the right quantity and total." project words: product
5. (todo) "Marking a todo done works now, and it sticks after a restart." project words: todo, done
6. (todo) "The done filter works now, for both true and false." project words: done, filter

The keywords sent in the "with" arm (the output of the shipped derivation for each app, in the order sent):

- notes: ["notes","auth","login","logout","register","server","store"]
- shop: ["cart","orders","carts","catalog","checkout","items","products","server"]
- todo: ["todos","auth","server","signup","token"]

Hits: 6 of 18 without keywords, 11 of 18 with.
Artifacts (a word said twice, or two known words stuck together): 6 without, 2 with.
Calls that took more than one try (any failed call is made again after 10 s, up to 3 tries in all; a third failure stops the run): 0.

Rule (fixed before the run): keywords are on by default only if hits with > hits without and artifacts with <= artifacts without.
Result: keywords are ON by default.

### Run it again

It costs about USD 0.01. Never run the tests with a bare `-- --ignored`: `tests/writer_live.rs` makes paid calls to the check-writer.

    node scripts/voice-measure.mjs gen <dir>        # the 18 clips and manifest.json (ffmpeg with the flite filter)
    META_API_KEY=... TORPOR_MUSE_STATE=<state dir> VOICE_MANIFEST=<dir>/manifest.json VOICE_ROWS=<dir>/rows.jsonl \
      cargo test --bin torpor-muse -- --ignored --exact suggest::voice::tests::measure_keywords --nocapture
    node scripts/voice-measure.mjs table <dir>/rows.jsonl

A clip and arm that already has a row in `VOICE_ROWS` is skipped, so a run that stopped halfway goes on where it was.

## Live run

2026-10-27 05:26 UTC, real Meta APIs and the real Torpor engine, keywords on (the default). The clip is `flite` (voice slt) saying "login works now for alice and bob", 2.6 s. The workspace is the small notes app of `tests/fixtures/writer/app`, with `bob` added. The test is `tests/voice_live.rs`; the engine's two turns (turn 1 "add a login to the notes app", turn 2 starting with the claim) were fed to it by the test, not by a Muse Code session.

    heard: "login works now for Alice and Bob."
    proven: Promise #1 "Login works for Alice and Bob" saved. Proof: passed 5 of 5 on the version from the start of turn 2 and failed 2 of 2 on the start of turn 1. In shadow for now: Torpor will say when it would have stopped the AI, but does not stop it yet. It starts to stop it after one real catch or three clean days, or when you run: torpor promise block 1

The transcription cost USD 0.00015 (audio 2640 ms, 5 keywords), the check-writer USD 0.005265. This was the first and only live run of it.

## Not done, and shortcuts

- Only synthetic voices were tried; no human speech, no microphone, English only.
- The keyword derivation has no stop-word list (`server` counts like any word), ignores plurals (the shop claims say "product", the list has "products"), and does not read test titles, saved checks or READMEs.
- The measurement is small (see above). It decides a default, it does not prove a gain.
- The header check does not decode audio, so a clip that is damaged after the header is reported by the API, not by Torpor.
