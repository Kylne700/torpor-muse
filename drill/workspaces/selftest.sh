#!/bin/sh
# Builder self-test. For every D*/build.sh: one good build into a fresh mktemp dir compared with
# its EXPECTED manifest (database files by name only), built twice for determinism, plus the guard
# refusals: non-empty dir, missing canary, wrong canary, unset or empty variable, wrong place,
# wrong name, wrong mode, symlink. A refusal must exit non-zero and leave the target untouched.
#   sh selftest.sh            run all builders        sh selftest.sh D01     only D01
#   sh selftest.sh --update   (re)write EXPECTED from a good build (review the diff, then commit)
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
ORACLE=$HERE/../oracles/files.mjs
BASE=${DRILL_TEST_BASE:-/tmp/claude-1000/T2}
mkdir -p "${BASE:?}"
UPDATE=0
if [ "${1:-}" = "--update" ]; then UPDATE=1; shift; fi
FAILS=0
ok() { printf 'ok   %s\n' "$*"; }
bad() { printf 'FAIL %s\n' "$*"; FAILS=$((FAILS + 1)); }
listing() { find "${1:?}" -mindepth 1 -maxdepth 1 | sort | tr '\n' ' '; }

# fresh: a new mktemp dir holding only the canary (its path is printed)
fresh() {
  _d=$(mktemp -d "${BASE:?}/drill.XXXXXX")
  printf '%s\n' "$_d" > "$_d/.drill-canary"
  printf '%s' "$_d"
}

# norm: drop database files from a manifest (they are compared by name, see dbnames)
norm() { grep -v -E '\.db(-wal|-shm)?"$' || true; }
dbnames() { for f in "${1:?}"/ws/data/*.db "${1:?}"/ws/data/*.db-wal "${1:?}"/ws/data/*.db-shm; do [ -e "$f" ] && basename "$f"; done; true; }

# in_dir DIR CMD...: run CMD with DIR as the working directory
in_dir() { (cd "${1:?}" && shift && "$@"); }

# must_refuse ID DESC CMD...: the builder run must fail
must_refuse() {
  _id=$1; _desc=$2; shift 2
  if "$@" >/dev/null 2>&1; then bad "$_id accepted: $_desc"; else ok "$_id refuses: $_desc"; fi
}

for dir in "$HERE"/D*/; do
  id=$(basename "$dir")
  if [ -n "${1:-}" ] && [ "$1" != "$id" ]; then continue; fi
  b=$dir/build.sh
  T=$(fresh)
  if sh "$b" "$T" >/dev/null 2>"$T.err"; then ok "$id builds"; else bad "$id build failed: $(head -c 300 "$T.err")"; fi
  if [ -e "$T/.drill-canary" ]; then bad "$id left the canary behind"; fi
  if [ ! -f "$T/ws/.synthetic" ] || [ ! -f "$T/.synthetic" ]; then bad "$id missing .synthetic markers"; fi
  node "$ORACLE" manifest "$T/ws" | norm > "$T.m1"
  dbnames "$T" > "$T.dbs"
  if [ "$UPDATE" -eq 1 ]; then
    cp "$T.m1" "$dir/EXPECTED"
    if [ -s "$T.dbs" ]; then cp "$T.dbs" "$dir/EXPECTED.dbs"; fi
    ok "$id EXPECTED written ($(wc -l < "$T.m1") lines)"
  else
    if cmp -s "$T.m1" "$dir/EXPECTED"; then ok "$id matches EXPECTED"; else bad "$id differs from EXPECTED"; fi
    if [ -f "$dir/EXPECTED.dbs" ]; then
      if cmp -s "$T.dbs" "$dir/EXPECTED.dbs"; then ok "$id database file names match"; else bad "$id database file names differ"; fi
    elif [ -s "$T.dbs" ]; then bad "$id has database files but no EXPECTED.dbs"; fi
  fi
  T2=$(fresh)
  sh "$b" "$T2" >/dev/null 2>&1 || true
  node "$ORACLE" manifest "$T2/ws" | norm > "$T2.m1"
  if cmp -s "$T2.m1" "$T.m1"; then ok "$id deterministic"; else bad "$id not deterministic"; fi

  R=$(fresh); : > "$R/extra-file"
  before=$(listing "$R")
  must_refuse "$id" "a non-empty dir" sh "$b" "$R"
  if [ "$(listing "$R")" != "$before" ]; then bad "$id touched the non-empty dir"; fi

  R=$(mktemp -d "${BASE:?}/drill.XXXXXX")
  must_refuse "$id" "a dir without the canary" sh "$b" "$R"
  if [ -n "$(listing "$R")" ]; then bad "$id touched the dir without a canary"; fi

  R=$(mktemp -d "${BASE:?}/drill.XXXXXX")
  printf 'wrong path\n' > "$R/.drill-canary"
  must_refuse "$id" "a canary with the wrong content" sh "$b" "$R"

  C=$(mktemp -d "${BASE:?}/drill.XXXXXX")
  must_refuse "$id" "an unset variable (no target argument)" in_dir "$C" sh "$b"
  must_refuse "$id" "an empty variable (empty target argument)" in_dir "$C" sh "$b" ""
  if [ -n "$(listing "$C")" ]; then bad "$id wrote into the cwd after a refusal"; fi

  V=$(mktemp -d /var/tmp/drill.XXXXXX); printf '%s\n' "$V" > "$V/.drill-canary"
  must_refuse "$id" "a dir outside /tmp" sh "$b" "$V"
  rm -f "$V/.drill-canary"; rmdir "$V"

  N=$(mktemp -d "${BASE:?}/other.XXXXXX"); printf '%s\n' "$N" > "$N/.drill-canary"
  must_refuse "$id" "a dir not named drill.*" sh "$b" "$N"
  rm -f "$N/.drill-canary"; rmdir "$N"

  M=$(fresh); chmod 755 "$M"
  must_refuse "$id" "a dir that is not mode 700" sh "$b" "$M"

  S=$(fresh); ln -s "$S" "$S.link"
  must_refuse "$id" "a symlink as target" sh "$b" "$S.link"
  rm -f "$S.link"
done
printf '%s failure(s)\n' "$FAILS"
[ "$FAILS" -eq 0 ]
