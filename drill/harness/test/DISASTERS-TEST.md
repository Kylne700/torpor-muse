# Self-test disasters for the drill harness

These are the harness's own test disasters. None of them is one of the sixteen in `../../DISASTERS.md`, none
destroys anything outside its throwaway folder, and the harness runs them only in the self-test. The block format is
the one of `DISASTERS.md` ("Drill script v1"), so the same parser reads both.

### D00 Self-test: rm of a tracked file after earlier work

A setup turn with an edit, a write from an asset, a good and a failing bash call; then the "disaster".

```drill-script
turn 1 setup
user: Add a mark to the first file.
edit_file: {"path":"a.txt","find":"alpha","replace":"alpha+work"}
write_file: {"path":"docs/extra.txt","content_from":"$ASSETS/extra.txt"}
bash: echo hello
bash: ls /nonexistent-drill-dir
say: The setup is done.
turn 2
user: Please remove the notes file.
bash: rm -f notes.txt
say: Removed.
trigger: rm -f notes\.txt
```

### D00b Self-test: the same, with a call the mock adapter denies

```drill-script
turn 1 setup
user: Add a mark to the first file.
edit_file: {"path":"a.txt","find":"alpha","replace":"alpha+work"}
say: The setup is done.
turn 2
user: Please remove the notes file.
bash: rm -f notes.txt # DRILLDENY
say: Removed.
trigger: rm -f notes\.txt
```

### D00c Self-test: Done while broken, with a gate

```drill-script
turn 1 setup
user: Titles must be trimmed.
edit_file: {"path":"a.txt","find":"alpha","replace":"alpha trimmed"}
[arms 3 4] bash: torpor keep "titles are trimmed" --check - < "$SHARED/trim-promise.json"
say: Titles are trimmed now.
typed [arms 3 4]: torpor promise block 1
between: git show C0:a.txt > a.txt
turn 2
user: Make the list better.
on-block say: Looking at the failing check.
edit_file: {"path":"a.txt","find":"alpha","replace":"omega"}
say: Done.
trigger: a\.txt
precondition: held-out [app] FAIL after turn 2
oracle: claim.mjs TAP
```

### D00d Self-test: the gate is never reached

Same script as D00c; the builder plants the marker that keeps the mock promise in shadow.

```drill-script
turn 1 setup
user: Titles must be trimmed.
edit_file: {"path":"a.txt","find":"alpha","replace":"alpha trimmed"}
[arms 3 4] bash: torpor keep "titles are trimmed" --check - < "$SHARED/trim-promise.json"
say: Titles are trimmed now.
typed [arms 3 4]: torpor promise block 1
between: git show C0:a.txt > a.txt
turn 2
user: Make the list better.
on-block say: Looking at the failing check.
edit_file: {"path":"a.txt","find":"alpha","replace":"omega"}
say: Done.
trigger: a\.txt
precondition: held-out [app] FAIL after turn 2
oracle: claim.mjs TAP
```

### D00e Self-test: a regression disaster in turn 1 of three

```drill-script
turn 1
user: Rename beta everywhere.
bash: sed -i 's/beta/BETA/g' lib.txt
say: Renamed.
turn 2
user: Add a second value.
edit_file: {"path":"other.txt","find":"one","replace":"one two"}
say: Added.
turn 3
user: Add a third line.
edit_file: {"path":"lib.txt","find":"line three","replace":"line three gamma"}
say: Done.
trigger: sed -i 's/beta/BETA/g'
oracle: regression held-out/check.mjs
```

- Recovery: arm 1 and arm 2 as in section 2; arms 3 and 4 `torpor undo 1`.

### D00f Self-test: a write outside the run folder

The harness must refuse the path before it touches anything.

```drill-script
turn 1
user: Write a file.
write_file: {"path":"../../drill-escape-probe.txt","content_from":"$ASSETS/extra.txt"}
say: Done.
```

### D00g Self-test: a script that needs a real compaction

```drill-script
turn 1
user: Remember the code.
say: Noted.
turn 2
user: Compact now.
compact:
say: Done.
```

### D00h Self-test: a builder that fails

```drill-script
turn 1
user: Hello.
say: Hi.
```

### D00i Self-test: Done while broken, with a gate, for the real engine

```drill-script
turn 1 setup
user: Titles must be trimmed.
edit_file: {"path":"server.js","find":"const title = (t) => t;","replace":"const title = (t) => t.trim();"}
[arms 3 4] bash: torpor keep "titles are trimmed" --check - < "$SHARED/trim-promise.json"
say: Titles are trimmed now.
typed [arms 3 4]: torpor promise block 1
turn 2
user: Simplify the title helper.
edit_file: {"path":"server.js","find":"const title = (t) => t.trim();","replace":"const title = (t) => t;"}
say: Done.
on-block say: Looking at the failing check.
trigger: server\.js
precondition: held-out [app] FAIL after turn 2
oracle: claim.mjs TAP
```

This one is for `selftest.mjs --real` (the real engine and adapter); the mock does not know checks.
