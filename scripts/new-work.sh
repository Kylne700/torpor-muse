#!/bin/sh
# Prints the event-work list from git and fails when this repo does not read as event-only work.
#   new-work.sh [--engine DIR] [--forbid DIR]...
# It fails (exit 1) when:
#   - the history is shallow, or a commit dated before the gate (2026-10-26T17:00:00Z) touches anything
#     but README.md and the licence files;
#   - the README block between the NEW-VS-OLD markers differs from NEW-VS-OLD.md;
#   - a tracked file is byte-identical (same git blob) to a file in the engine tree at the commit pinned in
#     engine.lock, or to a file in a --forbid directory.
# Not counted as an engine copy: LICENSE-MIT and LICENSE-APACHE (the licence texts).
# --engine DIR: a clone of the engine that has the pinned commit. Without it, the engine tree (no file
# contents) is fetched from the repo named in engine.lock. A --forbid path is read from where you run the script.
# Exit 2 = bad usage or lock.
# shortcut: only byte-identical copies are found, an edited copy passes; upgrade to a similarity check if that matters.
set -eu

GATE=2026-10-26T17:00:00Z # the conservative kickoff reading (some read 12:00 PM Eastern as 17:00 UTC)
TZ=UTC; export TZ         # git prints dates in this zone with --date=format-local
TAB=$(printf '\t')

usage() { echo "usage: new-work.sh [--engine DIR] [--forbid DIR]..." >&2; exit 2; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
trap 'exit 1' HUP INT TERM
: > "$tmp/forbid-dirs"
engine_dir=
while [ $# -gt 0 ]; do
  case $1 in
    --engine)
      [ $# -ge 2 ] || usage
      engine_dir=$(cd "$2" 2>/dev/null && pwd) || { echo "--engine: no such directory: $2" >&2; exit 2; }
      shift 2 ;;
    --forbid)
      [ $# -ge 2 ] || usage
      d=$(cd "$2" 2>/dev/null && pwd) || { echo "--forbid: no such directory: $2" >&2; exit 2; }
      echo "$d" >> "$tmp/forbid-dirs"
      shift 2 ;;
    *) usage ;;
  esac
done

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
[ -f engine.lock ] || { echo "no engine.lock in $root" >&2; exit 2; }
git rev-parse --git-dir >/dev/null 2>&1 || { echo "$root is not a git work tree" >&2; exit 2; }
lockval() { awk -v k="$1" '$1 == k { print $2; exit }' engine.lock; }
repo=$(lockval repo); commit=$(lockval commit)
if [ -z "$repo" ] || [ -z "$commit" ]; then echo "engine.lock lacks repo or commit" >&2; exit 2; fi

fails=0
bad() { echo "FAIL: $*" >&2; fails=$((fails + 1)); }

# 1. The event-work list. A commit counts at the earlier of its author and committer date.
[ "$(git rev-parse --is-shallow-repository)" = false ] ||
  bad "shallow clone: the history is incomplete, fetch all of it (git fetch --unshallow)"
echo "Event work: commits dated $GATE or later (UTC)"
git log --no-merges --reverse --date=format-local:%Y-%m-%dT%H:%M:%SZ --name-only --format='@@%h|%ad|%cd|%s' |
  awk -v gate="$GATE" '
    function flush(   t, i, line, code) {
      if (hash == "") return
      t = (ad < cd) ? ad : cd
      for (i = 1; i <= nf; i++) if (files[i] != "README.md" && files[i] != "LICENSE-MIT" && files[i] != "LICENSE-APACHE") code = 1
      if (code && first == "") first = hash " " t
      line = "  " hash " " t " " subj "\n"
      if (t < gate) {
        if (code) { line = line "    CODE BEFORE THE GATE:"; late = 1 } else line = line "    README + licences only:"
        for (i = 1; i <= nf; i++) line = line " " files[i]
        pre = pre line "\n"
      } else {
        for (i = 1; i <= nf; i++) line = line "    " files[i] "\n"
        ev = ev line
      }
      hash = ""
    }
    /^@@/ {
      flush()
      n = split($0, f, "|")
      hash = substr(f[1], 3); ad = f[2]; cd = f[3]
      subj = f[4]; for (i = 5; i <= n; i++) subj = subj "|" f[i]
      nf = 0
      next
    }
    $0 != "" { files[++nf] = $0 }
    END {
      flush()
      print "Before the gate (only README + licences are allowed):"
      printf "%s", (pre == "" ? "  none\n" : pre)
      print "At or after the gate (the event work):"
      printf "%s", (ev == "" ? "  none yet\n" : ev)
      print "First commit with files beyond README + licences: " (first == "" ? "none yet" : first)
      exit (late ? 1 : 0)
    }' || bad "a commit dated before $GATE touches files beyond README + licences"

# 2. The README block is NEW-VS-OLD.md, byte for byte.
start='<!-- NEW-VS-OLD:start -->'; end='<!-- NEW-VS-OLD:end -->'
if [ ! -f README.md ] || [ ! -f NEW-VS-OLD.md ]; then
  bad "README.md or NEW-VS-OLD.md is missing"
elif [ "$(grep -c -x -F "$start" README.md || true)" != 1 ] || [ "$(grep -c -x -F "$end" README.md || true)" != 1 ]; then
  bad "README.md needs exactly one $start line and one $end line"
else
  awk -v s="$start" -v e="$end" '$0 == e { on = 0 } on { print } $0 == s { on = 1 }' README.md > "$tmp/block"
  if cmp -s "$tmp/block" NEW-VS-OLD.md; then
    echo "ok: the README block equals NEW-VS-OLD.md"
  else
    bad "the README block differs from NEW-VS-OLD.md (first differences below)"
    diff NEW-VS-OLD.md "$tmp/block" | head -n 20 >&2 || true
  fi
fi

# 3. No tracked file is a pre-event file. Lists are "<git blob id><TAB><label>"; equal blob ids = equal bytes.
if [ -z "$engine_dir" ]; then
  engine_dir=$tmp/engine
  git init -q "$engine_dir"
  git -C "$engine_dir" fetch -q --depth 1 --filter=blob:none "$repo" "$commit" ||
    { echo "cannot fetch commit $commit from $repo" >&2; exit 2; }
fi
[ "$(git -C "$engine_dir" rev-parse --verify -q "$commit^{commit}" || true)" = "$commit" ] ||
  { echo "the engine at $engine_dir does not have commit $commit" >&2; exit 2; }
git -C "$engine_dir" ls-tree -r "$commit" | awk -F'\t' '{ split($1, a, " "); print a[3] "\tengine:" $2 }' > "$tmp/engine-blobs"

: > "$tmp/forbid-blobs"
while IFS= read -r d; do
  find "$d" -type f | LC_ALL=C sort > "$tmp/files"
  git hash-object --no-filters --stdin-paths < "$tmp/files" > "$tmp/hashes"
  paste "$tmp/hashes" "$tmp/files" >> "$tmp/forbid-blobs"
done < "$tmp/forbid-dirs"

git ls-files -s | awk -F'\t' '{ split($1, a, " "); print a[2] "\t" $2 }' > "$tmp/tracked"
awk -F'\t' '$2 != "LICENSE-MIT" && $2 != "LICENSE-APACHE"' "$tmp/tracked" > "$tmp/tracked-engine"

hits() { # hits <blobs> <tracked>: prints "<label><TAB><tracked path>" for every equal blob id
  LC_ALL=C sort -t "$TAB" -k1,1 "$1" > "$tmp/s1"
  LC_ALL=C sort -t "$TAB" -k1,1 "$2" > "$tmp/s2"
  LC_ALL=C join -t "$TAB" -o 1.2,2.2 "$tmp/s1" "$tmp/s2"
}
{ hits "$tmp/engine-blobs" "$tmp/tracked-engine"; hits "$tmp/forbid-blobs" "$tmp/tracked"; } > "$tmp/hits"
while IFS="$TAB" read -r label path; do
  bad "$path is byte-identical to $label: a pre-event file is not event work"
done < "$tmp/hits"
if [ ! -s "$tmp/hits" ]; then
  echo "ok: no tracked file is byte-identical to one of the $(wc -l < "$tmp/engine-blobs" | tr -d ' ') engine files at $commit or to one of the $(wc -l < "$tmp/forbid-blobs" | tr -d ' ') --forbid files"
fi

[ "$fails" -eq 0 ] || { echo "new-work: $fails check(s) failed" >&2; exit 1; }
echo "new-work: ok"
