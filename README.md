# Muse Code hooks + Muse Spark check-writer for Torpor (work in progress)

The goal: Muse Code hooks driving the Torpor engine, and Muse Spark writing the checks that stop "Done" while something the user said works is broken. This is work in progress for the Meta Global AI Developer Hackathon (October 26 to November 4, 2026).

Problem statement: "Push the Frontier" (software).

<!-- NEW-VS-OLD:start -->
## Built during the hackathon (judge this)
- In progress. A part is listed here only after it has run and the run was recorded. No part is listed yet.

Every code commit in this repo is dated 2026-10-26 17:00 UTC or later.
`scripts/new-work.sh` prints the event-work list from git.

## Not built during the hackathon (declared open-source base, not judged)
- The Torpor engine: github.com/Kylne700/torpor, tag `pre-event-final` (a2289f4). GitHub shows the tag created 2026-10-26T15:29:51Z and its release published 2026-10-26T15:29:56Z, both before kickoff. The pin is in `engine.lock`; `scripts/verify-engine.sh` checks it.
- Between tag `pre-event-base` (bf28031, created 2026-10-25T22:35:23Z) and `pre-event-final`, only two files differ: README.md (+2 lines) and THIRD-PARTY-NOTICES.md (a new file, +8725 lines). No engine source file or binary changed.
- Kickoff was 12:00 PM Eastern (16:00 UTC; some read it as 17:00 UTC). The first commit here (415b6cf, 2026-10-26T16:05:17Z) holds only the README and licence files; code starts at or after 17:00 UTC.
<!-- NEW-VS-OLD:end -->

## What is sent to Meta (planned)

Planned, not built. The check-writer would send two things to Muse Spark through the Meta Model API (planned model id `muse-spark-1.3`). The first is the words the user typed. The second is a capped excerpt of the project, taken only from allowlisted files and redacted before it is sent. It would run in the background after a prompt in Muse Code, or when the user runs `torpor-muse suggest` by hand.

The Torpor engine sends nothing to Meta. Its own README (Privacy section) says it makes no AI model calls and uploads nothing of the project. The same section says its only download is `torpor runner install`, which runs after the user confirms.

## Who builds this

AI coding agents (Claude Code) working for the author, Kylne700, write the code in this repo. Muse Code is planned to build the demo app, and that is not done yet.

## License

MIT OR Apache-2.0, the same as Torpor.
