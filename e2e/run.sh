#!/usr/bin/env bash
# Re-runs the end-to-end checks that need no model, against a real Muse Code install: A1 (hooks installed and removed
# next to a foreign hook), A2 (a healthy turn: doctor says the gate is live) and A8 (a hook Muse rejects, then repaired;
# the engine gone). The "echo" provider answers a prompt with the prompt itself: free, no key.
# Not part of scripts/accept.sh or CI. The full run (A1-A10, keyed checks included) and its findings: e2e/RESULTS.md.
#   usage: e2e/run.sh      MUSE=<muse binary, default: muse on PATH>  TORPOR_BIN=<pinned engine, required>
#                          TORPOR_MUSE=<adapter binary, default: target/release/torpor-muse>
# Every Muse process runs inside scripts/sandbox/muse-sandbox.sh with --net none (no network, the real home hidden,
# a cleared environment). All work happens in a fresh temp folder (printed at the end; delete it yourself).
set -u
here=$(cd "$(dirname "$0")" && pwd)
skip() { echo "SKIP: $*"; exit 0; }
MUSE=$(command -v "${MUSE:-muse}" 2>/dev/null) || skip "no Muse Code install (put muse on PATH or set MUSE=)"
[ -x "${TORPOR_BIN:-}" ] || skip "no engine (set TORPOR_BIN=<pinned torpor binary>)"
AD=${TORPOR_MUSE:-$here/../target/release/torpor-muse}
[ -x "$AD" ] || skip "no adapter binary (cargo build --release, or set TORPOR_MUSE=)"
SB=$here/../scripts/sandbox/muse-sandbox.sh
[ -x "$SB" ] || skip "no sandbox launcher (scripts/sandbox/muse-sandbox.sh)"
MUSE=$(readlink -f "$MUSE")

E=$(mktemp -d "${TMPDIR:-/tmp}/torpor-muse-e2e.XXXXXX") || exit 2
E=$(cd "$E" && pwd -P) # init writes the adapter's resolved path into the hook commands
H=$E/home W=$E/ws # the launcher makes HOME=$E/home
S=$H/.config/muse/settings.json
mkdir -p "$H/.config/muse" "$E/bin" "$W"
cp "$TORPOR_BIN" "$E/bin/torpor"    # copies: the engine is moved away below, the real one is never touched
cp "$AD" "$E/bin/torpor-muse"
AD=$E/bin/torpor-muse

box() { (cd "$W" && TORPOR_HOME="$E/store" TORPOR_BIN="$E/bin/torpor" "$SB" --run-dir "$E" --ro "$(dirname "$MUSE")" --chdir "$W" -- "$@"); }
box true > "$E/probe.txt" 2>&1 || skip "the sandbox does not start here (see $E/probe.txt; delete $E)"
run_tm() { box "$AD" "$@"; }
doctor() { run_tm doctor > "$E/doctor.txt" 2>&1; } # the exit status is doctor's
run_turn() {
  box "$MUSE" exec --provider echo --trust-workspace --contributor-mode off --approval-judge off \
    --disable-approval --max-model-steps 2 "$1" >> "$E/turns.txt" 2>&1
}
hooks() { python3 -I "$here/hooks.py" "$H/.local/share/muse/sessions" --last 1 "$@"; }
has() { grep -Eq "$1" "$2"; }
n=0 bad=0
check() { n=$((n + 1)); if "${@:2}"; then echo "PASS $1"; else echo "FAIL $1"; bad=$((bad + 1)); fi; }
quote() { grep -E "$1" "$2" | cut -c1-200 | sed 's/^/       /' | head -3; }

echo "muse: $(box "$MUSE" --version 2>&1 | head -1)  engine: $(box "$E/bin/torpor" version 2>&1 | head -1)  adapter: $(box "$AD" version 2>&1 | head -1)"
(cd "$W" && touch .synthetic && echo demo > README.md && git init -q && git config user.name e2e && git config user.email e2e \
  && git add -A && git -c commit.gpgsign=false commit -qm base)

# A1 + A2: a settings file with a foreign key and a foreign hook; install; one healthy turn; doctor
cat > "$S" <<EOF
{"schema_version":1,  "theme":"dark",
 "hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"sh -c 'cat >/dev/null; echo foreign-ups >> $E/foreign.txt'","timeout":5}]}]}}
EOF
cp "$S" "$E/settings.orig"
run_tm init > "$E/init.txt" 2>&1
check "A1 init adds our hook" has 'torpor-muse hook UserPromptSubmit' "$S"
check "A1 init keeps the foreign hook" has foreign-ups "$S"
cp "$S" "$E/settings.installed"
run_turn "say hello"
check "A2 the turn was answered" has '^echo: say hello' "$E/turns.txt"
check "A2 the foreign hook ran too" test -s "$E/foreign.txt"
hooks > "$E/hooks-ok.txt"
check "A2 Muse logged our Stop hook completed and none failed" sh -c "grep -Eq '^Stop +completed' '$E/hooks-ok.txt' && ! grep -Eq ' (failed|timed_out|blocked|cancelled) ' '$E/hooks-ok.txt'"
check "F4 Muse's own record of the first turn holds the start guide the model was given" grep -rq 'Torpor guide: stays valid' "$H/.local/share/muse/sessions"
doctor
rc=$?
check "A2 doctor exits 0" test $rc -eq 0
check "A2 doctor says the gate is live" has '^ok +gate live' "$E/doctor.txt"
check "A1 doctor names the Muse version" has '^ok +Muse Code [0-9]' "$E/doctor.txt"
quote '^ok +(gate live|Muse Code)' "$E/doctor.txt"

# A8: our UserPromptSubmit command replaced by a wrapper that prints {"torpor_drill_poison":1} and exits 0
cat > "$E/bin/poison.sh" <<'EOF'
#!/bin/sh
cat >/dev/null
printf '%s' '{"torpor_drill_poison":1}'
EOF
chmod +x "$E/bin/poison.sh"
sed "s#$AD hook UserPromptSubmit#$E/bin/poison.sh hook UserPromptSubmit#" "$E/settings.installed" > "$S"
check "A8 the poisoned command is in the settings" has 'poison.sh hook UserPromptSubmit' "$S"
run_turn "poison"
hooks --event UserPromptSubmit > "$E/hooks-poison.txt"
check "A8 Muse's log has the poisoned hook as failed" has '^UserPromptSubmit +failed.*torpor_drill_poison' "$E/hooks-poison.txt"
doctor
rc=$?
check "A8 doctor exits 1" test $rc -eq 1
check "A8 doctor reports the rejected hook" has '^FIX +Muse rejected a hook' "$E/doctor.txt"
quote '^FIX +Muse rejected a hook' "$E/doctor.txt"

# the repair: our command back, one more turn; the old failure is history (defect D1, found by this run)
cp "$E/settings.installed" "$S"
run_turn "after the repair"
doctor
rc=$?
check "D1 after the repair doctor exits 0" test $rc -eq 0
check "D1 ... and no longer reports the old failure" sh -c "! grep -q 'Muse rejected a hook' '$E/doctor.txt'"

# A8, second half: the engine binary gone from the hook's path
mv "$E/bin/torpor" "$E/bin/torpor.gone"
run_turn "no engine"
doctor
rc=$?
check "A8 no engine: doctor exits 1" test $rc -eq 1
check "A8 ... and names the missing engine" has '^FIX +the engine .* does not exist' "$E/doctor.txt"
quote '^FIX +the engine' "$E/doctor.txt"
mv "$E/bin/torpor.gone" "$E/bin/torpor"

# A1, second half: uninstall leaves the foreign file as it was
run_tm uninstall > "$E/uninstall.txt" 2>&1
check "A1 uninstall: the settings file is byte-identical to the original" cmp -s "$S" "$E/settings.orig"
check "A1 uninstall: none of our hooks is left" sh -c "! grep -q 'torpor-muse hook' '$S'"

echo "e2e: $((n - bad)) of $n checks passed. Evidence is in $E (delete it when done)."
[ "$bad" -eq 0 ]
