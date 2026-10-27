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

The check-writer sends two things to Muse Spark through the Meta Model API (model id `muse-spark-1.3`). The first is the words the user typed, whether or not they say that something works (the model decides that); anything that looks like a key is masked, and a prompt over 1500 characters is cut in the middle. The second is an excerpt of the project's code, capped (at most 6,000 bytes a file, about 14,000 in all) and redacted before it is sent: the names of the files that may go and the start of each. Only code files may go (`.js`, `.mjs`, `.cjs`, `.ts`, `.tsx`, `.jsx`, `.html`, `.ejs`, `.hbs`, `.pug`, `.py`, `.rb`, `.php`, `.go`, `.java`, `.rs`, `.cs`, `.vue`) and the manifests `package.json` and `tsconfig*.json`; in a git project only those git tracks or does not ignore, without git only those in the top folder. Nothing else goes: no data or notes (other `.json`, `.txt`, `.md`, `.csv`, `.sql`, database files), no README, no dotfile, nothing in `node_modules`, `dist`, `build`, `target`, `vendor`, `coverage`, `__pycache__`, `venv`, `out`, `tmp` or `logs`, and no file whose name holds `secret` or `credential`. Data written inside a code file (for example a `data.js` that holds the app's records) still goes with that file. It runs in the background after a prompt in Muse Code, or when the user runs `torpor-muse suggest` by hand.

To give it the key, put your Meta API key alone on one line in a file only you can read (`chmod 600`), and add the line `key_file = ~/path/to/that/file` to `~/.config/torpor-muse/config` (`chmod 600` that file too). The run that starts after a prompt in Muse Code takes the key from that file; `torpor-muse suggest` and `torpor-muse say` typed by hand also take `META_API_KEY`. Without the file that run does not start: `torpor-muse doctor` shows a `check-writer off` line with the reason, and the first prompt in Muse Code says so once.

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

Then `torpor-muse init` adds the hooks to Muse Code, `torpor-muse doctor` checks them (right after `init`, before one prompt has been sent in Muse Code and answered, it reports the gate as not proven live and exits 1, which is expected), and `torpor-muse uninstall` removes the hooks, the cache `init` wrote and every folder `init` made that is empty. It never deletes your data: the Torpor store (with your saved checks) and the spend ledger, results and logs in `~/.torpor/muse` stay, and uninstall says so when that folder still holds files; deleting it is your choice. The two programs in `~/.local/bin` stay too, and so do your settings file `~/.config/torpor-muse/config` and the engine's `~/.config/torpor`; to remove Torpor completely, delete those and the store `~/.torpor` as well. `torpor status` speaks for the engine only and can still say it is saving after `uninstall`: whether the hooks are in Muse Code, only `torpor-muse doctor` tells.
<!-- INSTALL:end -->

## Where to read Torpor's lines

Muse Code 1.4.13 shows the line Torpor prints after a turn only for a moment in its status bar and does not keep it in the conversation. In a terminal in the project folder, `torpor log` lists the turns and what each one changed, `torpor show N` the files and save points of turn N, and `torpor status` whether Torpor is saving the folder. `torpor undo N` undoes one turn, `torpor back N` puts the whole folder back to an earlier state, and `torpor help` lists the rest. When the check-writer saves a check that does not test everything you said, the line that announces the new promise names what it does not cover, and `torpor promise show N` prints that on its Why line when the line has room.

### The AI deleted or broke my database

`torpor undo` and a plain `torpor back` leave a database file (`.db`, `.sqlite`, `.sqlite3`) that is still there as it is. If a turn rewrote or deleted one, stop your app, then run `torpor back --id P --with-data`, where P is the start save point of that turn, unless your app was running when the turn began (the engine saves no database that an app holds open, so the start has none to bring back; Torpor's line then names the latest save point of that turn that has it, or says it has no copy): `torpor show N` prints it on its "Save points:" line ("Save points: 4 start · 7 end" gives P = 4), and after such a turn Torpor's line begins with the database that changed and this command, with P filled in. The AI gets it too, at the next prompt and at the start of every later Muse Code session in that folder (the last five such turns, until 14 days after the newest). It also puts every other file back to the start of that turn, so changes made after it are undone too.

### Hands-free: you talk, the AI runs Torpor

You can say "undo that" or "go back to how it was" in plain words and the AI runs the Torpor command itself. This part adds text for the AI and, at the end of a turn, at most one extra stop.

- At the start of every Muse Code session (also after resume, clear and compact) the AI gets a short guide, about 620 characters: which command answers "undo that", "go back", "what broke" and lost database data, to run it with `--dry-run` first and tell you in one sentence what it does, and which commands are yours alone (`off`, `on`, `gate`, `retain`, `forget`, `promise ok|drop|block`, `runner install`). Torpor refuses those before they run if the AI tries them.
- When a saved check fails after a turn, Torpor stops the AI once before it says Done. It names the promise (number and words), the turn that broke it when Torpor's own record points to one turn (otherwise it names `torpor find-break --promise N`), and what to do: fix the app and not the check, run `torpor check N` before saying it is fixed, and if the promise itself is wrong run `torpor promise dispute N "why"` and tell you. This happens once per turn: if the check still fails when that turn ends again, your status line still says so but the AI is not stopped again. A check Torpor calls flaky or has set aside, a subagent, and a turn the engine already stopped itself get no extra stop.
- When a turn deleted a file that existed before it, or changed or deleted a database file, Torpor stops the AI once too (in the same stop as a broken check, if both happened): it must tell you in one plain sentence what was deleted or changed and that you can get it back by asking. It does not undo anything unless you ask. For a database it is also given the `torpor back --id P --with-data` command, to run after the app has stopped. A turn that only added or edited files gets no stop, and neither does a turn whose changes came from a `torpor undo` or `torpor back` the AI ran.
- A command that would delete a database an app is still using is refused before it runs: the AI is told to stop the app first and then run the same command again, or to ask you. A database that a crash left in use stays refused.
- The gate is unchanged. None of this turns it on or off (`torpor gate on` and `off` stay yours) and the engine alone decides when a failing promise stops the AI by itself: with the gate on it does, and the extra stop is not added on top of it.

## License

MIT OR Apache-2.0, the same as Torpor.
