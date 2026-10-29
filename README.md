# Torpor for Muse Code

**You say what works. If the AI breaks it, the AI is told before it says "Done", and it can undo the damage.**

1. You say what works in Muse Code ("login works now"), typed or from a sound clip with `torpor-muse say`.
2. Muse Spark, Meta's model, writes a check from your words (Node apps for now).
3. At the end of a later turn that could have broken it, Torpor runs the check. If it fails, the AI is told before it says "Done", and "undo that" puts it back.

**Install** (Linux, macOS): `curl -fsSL https://raw.githubusercontent.com/Kylne700/torpor-muse/v0.1.0/install.sh | sh -eu && export PATH="$HOME/.local/bin:$PATH"`, and paste your [Meta API key](#add-your-meta-api-key) when it asks. After that you need no Torpor commands; you can just talk to the AI.

**Cost:** about half a US cent per prompt in a Node app (measured once: USD 0.005265). Unless a run stops first ([what is sent to Meta](#what-is-sent-to-meta)), the check-writer sends the cleaned words of each prompt (a prompt over 1,500 characters is cut to its first 1,000 and its last 500) to Meta's API for a yes/no answer to "does this say something works?". File names and up to about 14 KB of your code go in a request only when the answer is yes, or when that question gets no usable answer. It uses your own key, at most USD 2 a day unless you change the cap. The engine makes no AI calls.

**If the Torpor engine fails**, your Muse turn goes on. If that happens at the start of a session, at a prompt or at the end of a turn, you are told once per session that Torpor is not saving.

**Hackathon** (work in progress): the Muse Code hooks, the Muse Spark check-writer and voice were built during the Meta Global AI Developer Hackathon, October 26 to November 4, 2026. The Torpor engine (save points, undo) existed before and is not judged ([details](#built-during-the-hackathon-judge-this)).

Good to know:
- Optional commands: `torpor log` (what each turn changed), `torpor back N` (put the folder back to an earlier state), `torpor-muse doctor` (check that Torpor works). Everything else: `torpor --help`.
- A failing check tells the AI. It can stop the AI only after you run `torpor gate on` yourself, and then only a check that Torpor trusts (see "Tell, not stop" below).
- Speed: in the engine's own test (earlier build, no check running) no hook took longer than 128.9 ms ([record](https://github.com/Kylne700/torpor/blob/v2.3.1/NOTES-S1.md#L71-L89)).
- Cost details: [docs/voice.md](docs/voice.md).

<!-- CLIP: a 20-second clip goes here -->

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

<!-- INSTALL:start -->
## Install

`torpor-muse` connects Muse Code to the Torpor engine: Muse Code runs it at set moments (its hooks), it calls the pinned engine, and it asks Muse Spark to write the checks.

**One line:** `curl -fsSL https://raw.githubusercontent.com/Kylne700/torpor-muse/v0.1.0/install.sh | sh -eu && export PATH="$HOME/.local/bin:$PATH"`

It works only once tag v0.1.0 exists (cut at the freeze).

The install works on Linux (x64, arm64) and macOS (Apple silicon, Intel) and needs `curl` and `ca-certificates`. On a bare Debian or Ubuntu the first lines install them (as root, or with sudo). The block downloads torpor-muse v0.1.0 and the pinned Torpor engine (the commit and sha256 are in `engine.lock`), checks the sha256 of both, and puts them in `~/.local/bin`.

**Or read and paste the full script.** `install.sh` is the text between `sh -eu <<'EOF'` and `EOF` in this block:

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
# connect Muse Code and ask for the Meta API key; if that fails, the install still stands
trap '{ stty echo </dev/tty; } 2>/dev/null || :' INT # a Ctrl-C ends init, not this script; show typing again (the key prompt hid it)
{ e=$(PATH="$HOME/.local/bin:$PATH" "$HOME/.local/bin/torpor-muse" init 2>&1 >&3) ||
  echo "${e:-torpor-muse init stopped}; the install is done, run torpor-muse init later" >&2; } 3>&1
trap - INT
EOF
```

In a new terminal, if `torpor-muse` is not found, add `~/.local/bin` to your `PATH` (the block did that only for the shell it ran in).

With the GitHub CLI you can also check that the binary was built by this repository's release workflow: `gh attestation verify ~/.local/bin/torpor-muse -R Kylne700/torpor-muse`.

After the block:

- `torpor-muse init` adds the hooks to Muse Code.
- `torpor-muse doctor` checks them. Right after `init`, before one prompt has been sent in Muse Code and answered, it reports the gate as not proven live and exits 1. That is expected.
- `torpor-muse uninstall` removes the hooks, the cache `init` wrote and every folder `init` made that is empty. It never deletes your data. The Torpor store (with your saved checks) and the spend ledger, results and logs in `~/.torpor/muse` stay, and uninstall says so when that folder still holds files. Deleting it is your choice. The two programs in `~/.local/bin` stay too, and so do your settings file `~/.config/torpor-muse/config`, your key file and the engine's `~/.config/torpor`. To remove Torpor completely, delete those and the store `~/.torpor` as well.

`torpor status` speaks for the engine only and can still say it is saving after `uninstall`. Whether the hooks are in Muse Code, only `torpor-muse doctor` tells.
<!-- INSTALL:end -->

## Add your Meta API key

The installer (and `torpor-muse init` run in a terminal) asks for it. What you paste is not shown. It is saved alone on one line in `~/.config/torpor-muse/meta-api-key`, and `~/.config/torpor-muse/config` gets the line `key_file = ~/.config/torpor-muse/meta-api-key` in place of any old `key_file` line; only you can read either file. If a usable key file is already set, it says so and does not ask. Enter alone skips: Torpor saves and undoes without a key, and the check-writer stays off.

To set it by hand, put the key alone on one line in a file only you can read (`chmod 600`), and add the line `key_file = ~/path/to/that/file` to `~/.config/torpor-muse/config` (`chmod 600` that file too). The run that starts after a prompt in Muse Code takes the key from that file. With no usable key file it takes `META_API_KEY` from the environment Muse Code runs its hooks in, and with neither no run starts. `torpor-muse suggest` and `torpor-muse say` typed by hand take `META_API_KEY` when it is set, and the file otherwise.

With no key file, `torpor-muse doctor` shows a `check-writer off` line with the reason. With no key at all, the first prompt in each folder says so once for each reason (the line names it; a different reason is told once more).

Every claim ends in a saved check (the engine's `Torpor: new promise #N` line) or in one plain line that says why not, except the cases named at the end of this paragraph. The next prompt of that session says it (one line per prompt: a line that waits comes later, unless a newer run ends first and replaces it), once for each kind in the session: Meta refused the key or has no billing for it; the daily spend cap would be passed; the Meta API failed; the run took too long and was stopped; the model's answer could not be used, so no check was written; the check fails on the code as it is now, which can mean that what you claimed does not work yet or that the check is wrong; the Torpor engine did not finish (it timed out or ended in an error); any other end without a saved check, told in one plain sentence (the engine did not save the check, with its first reason, or every step needed a record already in your data); the run ended in an error of its own; an earlier run still held the project, so your last prompt was not looked at; or the run could not be started. In a folder without an app that starts from a script (`start`, `dev`, `serve` or `server` in `package.json`, or a `server.js`) the line comes once per folder, says that no check is written there, and says that save points and undo still work. Apart from the one-time notice in [What is sent to Meta](#what-is-sent-to-meta), these cases are normal and show nothing: a prompt that makes no claim, an answer saying the claim cannot be checked, a prompt where Torpor is not saving, and a prompt whose turn already has a check from your words.

## Where to read Torpor's lines

Torpor's own lines and commands call a check a "promise" (`torpor promise list`). This README says "check".

In our own use of Muse Code 1.4.13, the line Torpor prints after a turn showed only for a moment in its status bar and was not kept in the conversation. In a terminal in the project folder:

- `torpor log` lists the turns and what each one changed.
- `torpor show N` shows the files and save points of turn N.
- `torpor status` says whether Torpor is saving the folder.
- `torpor undo N` undoes one turn.
- `torpor back N` puts the whole folder back to an earlier state.
- `torpor help` lists the rest.

Torpor keeps every save point for two days (less in a project whose records pass 2 GB). After that it keeps only the last one of each day (see "How long save points last" in the engine's README), so for an older turn `torpor undo N` and `torpor back` may find no save point to go back to.

When the check-writer saves a check that does not test everything you said, the line that announces the new check, in the session that asked for it, names what it does not cover. `torpor promise show N` prints that on its Why line when the line has room.

### The AI deleted or broke my database

Asking the AI is enough, for example "bring my database back": it gets the command below and is told to run it when you ask.

`torpor undo` and a plain `torpor back` leave a database file (`.db`, `.sqlite`, `.sqlite3`) that is still there as it is. If a turn rewrote or deleted one:

1. Stop your app.
2. Run `torpor back --id P --with-data`, where P is the start save point of that turn, unless your app was running when the turn began (see below). `torpor show N` prints P on its "Save points:" line ("Save points: 4 start · 7 end" gives P = 4).

After such a turn, Torpor's line names the database that changed and gives this command, with P filled in. The command also puts every other file back to the state of that save point, so changes made after it are undone too.

If your app was running when the turn began, the start may have no copy to bring back: the engine does not save a database that is in use (its `-wal` file holds data, or a `-journal` file is there). Torpor's line then names the latest save point of that turn that has it, or says it has no copy.

The AI gets the same command too, in a note at the next prompt and at the start of every later Muse Code session in that folder (the last five such turns, until 14 days after the newest). The note says to run it only when you ask. The command works only while that save point is still there (see above).

### Hands-free: you talk, the AI runs Torpor

You can say "undo that" or "go back to how it was" in plain words, and the AI runs the Torpor command itself. This part adds text for the AI, for example a guide at the start of a session and, at the end of a turn, at most one extra note.

- **The guide.** At the start of every Muse Code session (also after resume, clear and compact; when Muse installs a compaction without running a start hook, the AI gets the same once, with its next prompt or tool result) the AI gets a short guide: 656 characters, and 11 more plus the engine's full path when the guide names it (743 in our test, where the path had 76 characters). It says which command answers "undo that", "go back", "what broke" and lost database data. For undo and back it says to act only when you ask: run `--dry-run` first, tell you in one sentence what it changes, then run it, because your ask is the yes. It says to ask you first only if the dry run also undoes other work or overwrites an existing database. It also says which commands are yours alone (`off`, `on`, `gate`, `retain`, `forget`, `undo --force`, `promise ok|drop|block|show --spec`, `runner install`). Torpor refuses those before they run if the AI tries them.
- **A broken check.** When a saved check fails after a turn, Torpor tells the AI once before it says Done. The note names the check (number and words). It names the turn that broke it when Torpor's own record points to one turn (otherwise it names `torpor find-break --promise N`). It says what to do: fix the app and not the check, run `torpor check N` before saying it is fixed, and, if the check itself is wrong, run `torpor promise dispute N "why"` and tell you. This happens once per turn: if the check still fails when that turn ends again, your status line still says so but the AI is not told again. A check Torpor calls flaky or has set aside, a subagent, and a turn the engine already stopped itself get no extra note.
- **A deleted file or database.** When a turn deleted a file that existed before it, or deleted or lost a database file, Torpor tells the AI once too at the end of the turn (in the same note as a broken check, if both happened). The note tells the AI to tell you, in one plain sentence, what was deleted or changed and that you can get it back by asking, and not to undo anything unless you ask. For a database it is also given the `torpor back --id P --with-data` command, to run after the app has stopped, when a save point has the database. It is told to use `--dry-run` first, to run the command only if you ask, and that if the dry run brings back only what was deleted, your ask is enough. A file that was only moved or renamed usually does not count as deleted: Torpor matches it to a new file with the same extension and the same number of lines. A binary or large file, or a file moved and edited in the same turn, can still be told as deleted. A database that only changed (a migration, a seed, a test run, a running app) gets Torpor's line to you and no end-of-turn note to the AI; the AI gets only the later database note described above. A turn that only added or edited files gets no such note, and neither does a turn whose changes came from a `torpor undo` or `torpor back` the AI ran.
- **A database an app is using.** A Bash command that would delete, move or overwrite a database in the project folder that is in use (its `-wal` file holds data, or a `-journal` file is there) is refused before it runs: the AI is told to stop the app first and then run the same command again, or to ask you. It looks at `rm`, `mv`, `cp`, a `>` redirect, `truncate`, `find -delete`, `git clean` and the like; a command it cannot read (a script, `sed -i`, SQL such as `DROP TABLE`) is not seen. If no app is running and a crash left files behind, the AI is told to open each database once with a read (`sqlite3 <db> 'SELECT count(*) FROM sqlite_master;'`, or the same through `python3`), which lets SQLite tidy the leftover files, and then to run the same command again. That clears the leftovers of a killed app in WAL mode and of a crash in the middle of a write. It does not clear the journal file that a clean close leaves in PERSIST or TRUNCATE mode, or a database that another program still has open: the command stays refused, and the AI is told to ask you.
- **Files outside the project folder.** Torpor also stops a shell command that would delete or move files outside the project folder (such as `rm -rf ~/old-notes`), because it saves only the project folder and could not bring them back; the AI is told to ask you first. Temp folders such as `/tmp` are not stopped, and a command it cannot read (a script run by name, `rsync --delete`) is not seen. If the AI writes `rm -rf ~` into a file, it gets a one-line warning and nothing is stopped.
- **A false Done.** Torpor also catches a false "Done": if the AI ends a turn saying the tests pass or the build works, but no such run happened in this session (if it began new, not by a resume), the last one failed, or code changed after the last pass, the AI is told so once and asked to run it or tell you. It sees only what Muse reports (not the result of a piped test command, edits made through bash, or tests you ran yourself), and when it is unsure it stays silent.
- **Tell, not stop.** A broken check is told to the AI before it says Done; it does not stop the AI. To let checks stop it, run `torpor gate on` in your own terminal (the "gate" is the switch for that). Even then a new check only tells at first: it starts to stop the AI after one real catch or three clean days, or after you run `torpor promise block N`, and some checks never stop it. Torpor's own default is on, so the first Muse Code session in a folder that Torpor saves runs `torpor gate off` there once. A `gate on` you type after that stays on. `torpor-muse doctor` shows which it is.

## Voice promises

`torpor-muse say <clip.wav>` is the spoken version of `torpor-muse suggest`. The clip goes to Meta's Muse Voice Transcribe API, the words it heard are printed, and the same words go to the check-writer as if they had been typed (same exit codes). It takes a WAV file; there is no microphone code.

Privacy: the audio file is sent to Meta's Voice Transcribe API; Torpor stores no audio. By default, up to 24 words from the names of your code files and the URL paths in your code are sent along with the audio, so that the transcription favours your own words. `--no-keywords` sends none. A measurement on 18 synthetic clips chose that default (11 clips heard every project word with the keywords, 6 without). [docs/voice.md](docs/voice.md) has every transcript and the limits.

A live run, 2026-10-27 05:26 UTC: a synthetic voice (flite) says "login works now for alice and bob" to the small demo app of `tests/fixtures/writer/app`, with the real Torpor engine and the real Meta APIs. The test (`tests/voice_live.rs`) adds `bob` to a copy of the app and feeds the engine's two turns itself, so this is not a Muse Code session; see [docs/voice.md](docs/voice.md).

    heard: "login works now for Alice and Bob."
    proven: Promise #1 "Login works for Alice and Bob" saved. Proof: passed 5 of 5 on the version from the start of turn 2 and failed 2 of 2 on the start of turn 1. In shadow for now: Torpor will say when it would have stopped the AI, but does not stop it yet. It starts to stop it after one real catch or three clean days, or when you run: torpor promise block 1

It cost USD 0.00015 for the transcription and USD 0.005265 for the check-writer.

That run was recorded before Torpor started to switch the gate off at the first Muse Code session (see Hands-free). With the gate off, the check-writer's line stops after "failed 2 of 2 on the start of turn 1." and then reads: A broken check is told to the AI before it says Done; it does not stop the AI. To let checks stop it, run `torpor gate on` in your own terminal.

## What is sent to Meta

The check-writer runs in the background after a prompt in Muse Code, or when you run `torpor-muse suggest` by hand. One run makes up to three requests to Muse Spark through the Meta Model API (model id `muse-spark-1.3`). Nothing is sent when the run cannot go on, for example when there is no key, when `torpor off` is on in the folder (for a run that a prompt starts), or when another run holds the folder. The first time the check-writer is about to send a prompt from a folder, Torpor tells you once (rarely one prompt late) what it sends and that `torpor off` in that folder stops it, and stops Torpor's saving there too.

1. The words you typed, cleaned and redacted, for a yes/no question: does this say something works? A prompt over 1,500 characters is cut in the middle: its first 1,000 and its last 500 characters go, with `[...]` between them. No file name and no code go with them. If the answer is no, the run ends there and nothing more is sent.
2. Only when the answer is yes, or when the first request got no usable answer (a failed call, a timeout, an answer that cannot be read): the same words again and a look at the project's code, redacted: a list of up to 120 names of files that may go, then the start of those files: `package.json` first, then entry files such as `server.js`, then top-level files and routes, test files last (except those named like an entry file, such as `app.test.js`). At most 6,000 bytes of one file and 300 characters of one line go, and about 14,000 bytes in all, the list included. A file over 64,000 bytes is not read. The request also names the app's start scripts (`start`, `dev`, `serve` or `server` from `package.json`, or `node server.js`).
3. A second try of item 2, only when the first try failed: a call that failed in a way worth another try, an answer that could not be used, or a check the engine turned down for a reason a new check could fix. It sends the same words and the same look at the code again. After an answer that could not be used or a check the engine turned down, it also sends the model's earlier answer (cleaned and cut to 3,000 characters) and the reason the parser or the engine gave.

Only code files may go (`.js`, `.mjs`, `.cjs`, `.ts`, `.tsx`, `.jsx`, `.html`, `.ejs`, `.hbs`, `.pug`, `.py`, `.rb`, `.php`, `.go`, `.java`, `.rs`, `.cs`, `.vue`) and the manifests `package.json` and `tsconfig*.json`. In a git project only files git does not ignore may go (tracked, or not yet added); without git only those in the top folder (a plain file name listed in its `.gitignore` is left out). Nothing else goes: no data or notes (other `.json`, `.txt`, `.md`, `.csv`, `.sql`, database files), no README, nothing whose path has a part that starts with a dot, nothing in `node_modules`, `dist`, `build`, `target`, `vendor`, `coverage`, `__pycache__`, `venv`, `out`, `tmp` or `logs`, no `.min.js` or `.d.ts` file, and no file whose name holds `secret` or `credential`. Data written inside a code file (for example a `data.js` that holds the app's records) still goes with that file.

Redaction is best effort, by name and by shape. It replaces with `[redacted]` the value after a name such as `secret`, `token`, `apikey`, `private_key`, `credential`, `authorization` or `password` (a value in quotes always, one without quotes only if it has 6 or more characters), the user and password in a URL, the word of 8 or more characters after `Bearer` or `Basic`, and anything shaped like a key (a string of 24 or more characters with at least 2 digits and 8 letters, or one that starts like a known token, has a digit and goes on for 8 or more characters: `sk-`, `AKIA`, `AIza`, `LLM_`, `eyJ`, and the prefixes of GitHub, GitLab and Slack tokens); a PEM key block is cut out. Only in a folder with a `.synthetic` file (made-up content, for tests and demos) a password stays, so that the model can use a demo login. A secret in another shape can get through: keep real secrets out of code files.

`torpor-muse say <clip.wav>` first sends the audio file to Meta's Voice Transcribe API (model `muse-voice-transcribe-1.0`) and, unless `--no-keywords` is given, up to 24 words from the names of the project's code files and the URL paths in its code. The words that come back then go to the check-writer as typed words, so the requests above are made as well.

The Torpor engine sends nothing to Meta. Its own README (Privacy section) says it makes no AI model calls and uploads nothing of the project. The same section says its only download is `torpor runner install`, which runs after you confirm.

Muse's model can read Torpor's stored checks; the engine refuses only changes to its store.

To stop only the check-writer and keep Torpor saving, take its key away: delete the `key_file` line from `~/.config/torpor-muse/config` (or put a `#` in front of it), and do not have `META_API_KEY` set in the environment Muse Code runs its hooks in. The next prompt then starts no run and sends nothing to Meta, the first prompt in each folder says, once for each reason, that the check-writer is off, and save points, undo and the notes to the AI go on as before. `day_cap_usd = 0` in the same file also sends nothing, but a run still starts after each prompt, and each session hears once that the daily spend cap would be passed. `torpor off` in a project also ends the runs that a prompt starts there (such a run asks the engine first and stops before it reads your key or sends anything; a `torpor-muse suggest` you type by hand does not ask), but it stops saving too.

## Who builds this

AI coding agents (Claude Code) working for the author, Kylne700, write the code in this repo, except [`demo/`](demo/). The three files there (`app/server.js`, `pane.js` and `README.md`) were written by Muse Code (model `muse-spark-1.3-contributor`) from English prompts. No demo code was pasted into a prompt and no line was edited by a person (replaying the logged file writes gives the committed files byte for byte). The prompt for `demo/README.md` dictated much of its wording: six of its 20 non-empty lines are in the prompt word for word, the last one among them. [`demo/transcripts/`](demo/transcripts/) holds every prompt word for word and the session logs, redacted and filtered: Meta's own system and developer messages, the tool definitions, encrypted reasoning blobs, other bookkeeping events and the content of the two scratch scripts are left out, and tool results are cut at 6000 characters.

## License

MIT OR Apache-2.0, the same as Torpor.
