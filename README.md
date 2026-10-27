# Muse Code hooks + Muse Spark check-writer for Torpor (work in progress)

The goal: Muse Code hooks driving the Torpor engine, and Muse Spark writing the checks that stop "Done" while something the user said works is broken. This is work in progress for the Meta Global AI Developer Hackathon (October 26 to November 4, 2026).

Problem statement: "Push the Frontier" (software).

<!-- NEW-VS-OLD:start -->
## Built during the hackathon (judge this)
- In progress. A part is listed here only after it has run and the run was recorded.
- Voice promises: `torpor-muse say <clip.wav>` sends a spoken claim to Meta's Voice Transcribe API and hands the words it heard to the check-writer as if they had been typed. A keyword measurement chose its default, and one live run (2026-10-27) is recorded in the README and in `docs/voice.md`.

Every code commit in this repo is dated 2026-10-26 17:00 UTC or later.
`scripts/new-work.sh` prints the event-work list from git.

## Not built during the hackathon (declared open-source base, not judged)
- The Torpor engine: github.com/Kylne700/torpor, tag `pre-event-final` (a2289f4). GitHub shows the tag created 2026-10-26T15:29:51Z and its release published 2026-10-26T15:29:56Z, both before kickoff. The pin is in `engine.lock`; `scripts/verify-engine.sh` checks it.
- Between tag `pre-event-base` (bf28031, created 2026-10-25T22:35:23Z) and `pre-event-final`, only two files differ: README.md (+2 lines) and THIRD-PARTY-NOTICES.md (a new file, +8725 lines). No engine source file or binary changed.
- Kickoff was 12:00 PM Eastern (16:00 UTC; some read it as 17:00 UTC). The first commit here (415b6cf, 2026-10-26T16:05:17Z) holds only the README and licence files; code starts at or after 17:00 UTC.
<!-- NEW-VS-OLD:end -->

## Voice promises

`torpor-muse say <clip.wav>` is the spoken version of `torpor-muse suggest`. The clip goes to Meta's Muse Voice Transcribe API, the words it heard are printed, and the same words go to the check-writer as if they had been typed (same exit codes). It takes a WAV file; there is no microphone code.

Privacy: the audio file is sent to Meta's Voice Transcribe API; Torpor stores no audio. By default, up to 24 words from the names of your code files and the URL paths in your code are sent along with the audio, so that the transcription favours your own words; `--no-keywords` sends none. A measurement on 18 synthetic clips chose that default (11 clips heard every project word with the keywords, 6 without). [docs/voice.md](docs/voice.md) has every transcript and the limits.

A live run, 2026-10-27 05:26 UTC: a synthetic voice (flite) says "login works now for alice and bob" to the small demo app of `tests/fixtures/writer/app`, with the real Torpor engine and the real Meta APIs. The test is `tests/voice_live.rs`.

    heard: "login works now for Alice and Bob."
    proven: Promise #1 "Login works for Alice and Bob" saved. Proof: passed 5 of 5 on the version from the start of turn 2 and failed 2 of 2 on the start of turn 1. In shadow for now: Torpor will say when it would have stopped the AI, but does not stop it yet. It starts to stop it after one real catch or three clean days, or when you run: torpor promise block 1

It cost USD 0.00015 for the transcription and USD 0.005265 for the check-writer.

## What is sent to Meta (planned)

Planned, not built. The check-writer would send two things to Muse Spark through the Meta Model API (planned model id `muse-spark-1.3`). The first is the words the user typed. The second is a capped excerpt of the project, taken only from allowlisted files and redacted before it is sent. It would run in the background after a prompt in Muse Code, or when the user runs `torpor-muse suggest` by hand.

The Torpor engine sends nothing to Meta. Its own README (Privacy section) says it makes no AI model calls and uploads nothing of the project. The same section says its only download is `torpor runner install`, which runs after the user confirms.

Muse's model can read Torpor's stored checks; the engine refuses only changes to its store.

## Who builds this

AI coding agents (Claude Code) working for the author, Kylne700, write the code in this repo. Muse Code is planned to build the demo app, and that is not done yet.

## License

MIT OR Apache-2.0, the same as Torpor.
