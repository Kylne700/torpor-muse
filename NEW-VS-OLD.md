## Built during the hackathon (judge this)
- A part is listed here only after it has run and the run was recorded.
- These ran in the L4c proof with real Muse Code; each line names its row in [e2e/PROOF.md](e2e/PROOF.md). Saving, undoing and running a saved check are the engine's work and are not claimed:
  - Save points and "undo that": the hooks make the engine save points, and on "undo that" the AI ran the engine's undo and the files came back 14.47 s and 20.01 s later (E5 cases 2 and 5, E6).
  - Check-writer: Muse Spark writes a check from "X works" and the engine saves it. Some claims got no saved check, and no pass line covers that (E3, E4).
  - Stop note: at the end of a turn the engine ran the saved check; when it failed, the Stop hook told the AI, and the AI fixed it in the same turn with 0 user messages (met on one reading, see "Measured" in the README: E5 case 1, E7 (b)).
  - Database guard: a command that would delete a database an app holds open was refused (E5 case 4). After a deleted database the AI was given the engine's restore command and ran it when asked, with no question to the user (E5 case 3, met on one reading).
  - Outside-the-folder guard: an `rm -rf` of a folder outside the project was refused and the folder was untouched (E8).
  - Compaction note: after a compaction the AI got a text that names the saved check, 9 of 9 times by the count fixed before the run, 5 of 9 if only a session-start block counts (E7 (a)).
  - Hands-free guide: 711 characters at the start of each of the two clean sessions (E2). E2 as a whole was not met, because a separate 328-character note reached the AI on a clean turn (see "Measured" in the README).
  - False-Done check: built and covered by tests, but E9 did not trigger it in either proof run (the AI did not write a false "Done"), so the proof does not show it catching one (E9).
- Voice promises: `torpor-muse say <clip.wav>` sends a spoken claim to Meta's Voice Transcribe API and hands the words it heard to the check-writer as if they had been typed. A keyword measurement chose its default, and one live run (2026-10-27) is recorded in the README and in `docs/voice.md`.

Every code commit in this repo is dated 2026-10-26 17:00 UTC or later.
`scripts/new-work.sh` prints the event-work list from git.

## Not built during the hackathon (declared open-source base, not judged)
- The Torpor engine: github.com/Kylne700/torpor, tag `pre-event-final` (a2289f4). GitHub shows the tag created 2026-10-26T15:29:51Z and its release published 2026-10-26T15:29:56Z, both before kickoff. The pin is in `engine.lock`; `scripts/verify-engine.sh` checks it.
- Between tag `pre-event-base` (bf28031, created 2026-10-25T22:35:23Z) and `pre-event-final`, only two files differ: README.md (+2 lines) and THIRD-PARTY-NOTICES.md (a new file, +8725 lines). No engine source file or binary changed.
- Kickoff was 12:00 PM Eastern (16:00 UTC; some read it as 17:00 UTC). The first commit here (415b6cf, 2026-10-26T16:05:17Z) holds only the README and licence files; code starts at or after 17:00 UTC.
