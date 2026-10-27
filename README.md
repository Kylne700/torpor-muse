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

A live run, 2026-10-27 05:26 UTC: a synthetic voice (flite) says "login works now for alice and bob" to the small demo app of `tests/fixtures/writer/app`, with the real Torpor engine and the real Meta APIs. The test (`tests/voice_live.rs`) adds `bob` to a copy of the app and feeds the engine's two turns itself, so this is not a Muse Code session; see [docs/voice.md](docs/voice.md).

    heard: "login works now for Alice and Bob."
    proven: Promise #1 "Login works for Alice and Bob" saved. Proof: passed 5 of 5 on the version from the start of turn 2 and failed 2 of 2 on the start of turn 1. In shadow for now: Torpor will say when it would have stopped the AI, but does not stop it yet. It starts to stop it after one real catch or three clean days, or when you run: torpor promise block 1

It cost USD 0.00015 for the transcription and USD 0.005265 for the check-writer.

## What is sent to Meta

The check-writer sends two things to Muse Spark through the Meta Model API (model id `muse-spark-1.3`). The first is the words the user typed, whether or not they say that something works (the model decides that); anything that looks like a key is masked, and a prompt over 1500 characters is cut in the middle. The second is a capped excerpt of the project, taken only from allowlisted files and redacted before it is sent. It runs in the background after a prompt in Muse Code, or when the user runs `torpor-muse suggest` by hand.

`torpor-muse say <clip.wav>` first sends the audio file to Meta's Voice Transcribe API (model `muse-voice-transcribe-1.0`) and, unless `--no-keywords` is given, up to 24 words from the names of the project's code files and the URL paths in its code. The words that come back then go to the check-writer as typed words, so the two things above are sent as well.

The Torpor engine sends nothing to Meta. Its own README (Privacy section) says it makes no AI model calls and uploads nothing of the project. The same section says its only download is `torpor runner install`, which runs after the user confirms.

Muse's model can read Torpor's stored checks; the engine refuses only changes to its store.

## Who builds this

AI coding agents (Claude Code) working for the author, Kylne700, write the code in this repo. Muse Code is planned to build the demo app, and that is not done yet.

<!-- INSTALL:start -->
## Install

Linux (x64, arm64) and macOS (Apple silicon, Intel). It needs `curl` and `ca-certificates`; on a bare Debian or Ubuntu the first lines install them (as root, or with sudo). The block downloads torpor-muse v0.1.0 and the pinned Torpor engine (the commit and sha256 in `engine.lock`), checks the sha256 of both, and puts them in `~/.local/bin`.

```sh
if ! command -v curl >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1; then
  s=; [ "$(id -u)" -eq 0 ] || s=sudo
  $s apt-get update -qq && $s apt-get install -y -qq curl ca-certificates
fi
sh -eu <<'EOF' && export PATH="$HOME/.local/bin:$PATH"
rel=https://github.com/Kylne700/torpor-muse/releases/download/v0.1.0
eng=https://raw.githubusercontent.com/Kylne700/torpor/a2289f4c3cdc7779f18cda56f95c607bb01a88cc/dist
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) p=linux-x64 e=0048fc3b7ee63f3d77c9f3cb63941176b30ddd427f7f6dd8dae929241c7f2e3f ;;
  Linux-aarch64 | Linux-arm64) p=linux-arm64 e=0e6e61341e735319b8a5d857031324c192ea675b8d9b4fb29e96672dc6039c7d ;;
  Darwin-x86_64) p=darwin-x64 e=8b4bbf292026537b0b84db69f3c0c0463650ca76e3d21dd93863d51c40c184dc ;;
  Darwin-arm64) p=darwin-arm64 e=0ffd3a1c05f8ddefef4ffe8c04da6c4118625cb3a5465af94a359624650a948b ;;
  *) echo "no torpor-muse build for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
check() { if command -v sha256sum >/dev/null 2>&1; then sha256sum -c "$1"; else shasum -a 256 -c "$1"; fi; }
d=$(mktemp -d)
trap 'rm -rf "$d"' EXIT
cd "$d"
curl -fsSL -o torpor "$eng/$p/torpor"
echo "$e  torpor" > torpor.sha256
check torpor.sha256
curl -fsSL -o "torpor-muse-$p" "$rel/torpor-muse-$p"
curl -fsSL -o SHA256SUMS "$rel/SHA256SUMS"
grep " torpor-muse-$p\$" SHA256SUMS > torpor-muse.sha256
check torpor-muse.sha256
mkdir -p "$HOME/.local/bin"
install -m 755 torpor "$HOME/.local/bin/torpor"
install -m 755 "torpor-muse-$p" "$HOME/.local/bin/torpor-muse"
EOF
```

In a new terminal, if `torpor-muse` is not found, add `~/.local/bin` to your `PATH` (the block did that only for the shell it ran in).

With the GitHub CLI you can also check that the binary was built by this repository's release workflow: `gh attestation verify ~/.local/bin/torpor-muse -R Kylne700/torpor-muse`.

Then `torpor-muse init` adds the hooks to Muse Code, `torpor-muse doctor` checks them, and `torpor-muse uninstall` removes the hooks, the cache `init` wrote and every folder `init` made that is empty. It never deletes your data: the Torpor store (with your saved checks) and the spend ledger, results and logs in `~/.torpor/muse` stay, and uninstall says so when that folder still holds files; deleting it is your choice. The two programs in `~/.local/bin` stay too.
<!-- INSTALL:end -->

## License

MIT OR Apache-2.0, the same as Torpor.
