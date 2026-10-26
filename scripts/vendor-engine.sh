#!/bin/sh
# Route (ii): puts a copy of the engine, as of the commit pinned in engine.lock, into pre-event-engine/,
# a folder labelled "pre-event, not judged". Dry run by default: it only prints what it would do.
#   vendor-engine.sh [--apply] [--engine DIR]
# --apply refuses if pre-event-engine/ exists. It writes the folder (git archive of the locked commit), checks the
# copy is byte-identical (the files give the same git tree id as the commit: paths, modes and bytes), then writes
# LABEL.md. If the check fails it removes the folder it just made. It never stages, commits or pushes.
# --engine DIR: a clone of the engine that has the pinned commit; without it, a shallow clone of the tag
# named in engine.lock is made in a temp directory.
# Exit 0 = ok, 1 = check failed, 2 = bad usage or lock.
set -eu

usage() { echo "usage: vendor-engine.sh [--apply] [--engine DIR]" >&2; exit 2; }
apply=0
engine_dir=
while [ $# -gt 0 ]; do
  case $1 in
    --apply) apply=1; shift ;;
    --engine)
      [ $# -ge 2 ] || usage
      engine_dir=$(cd "$2" 2>/dev/null && pwd) || { echo "--engine: no such directory: $2" >&2; exit 2; }
      shift 2 ;;
    *) usage ;;
  esac
done

root=$(cd "$(dirname "$0")/.." && pwd)
lock=$root/engine.lock
target=$root/pre-event-engine
[ -f "$lock" ] || { echo "no engine.lock in $root" >&2; exit 2; }
lockval() { awk -v k="$1" '$1 == k { print $2; exit }' "$lock"; }
repo=$(lockval repo); tag=$(lockval tag); commit=$(lockval commit); created=$(lockval tag-created)
if [ -z "$repo" ] || [ -z "$tag" ] || [ -z "$commit" ] || [ -z "$created" ]; then
  echo "engine.lock lacks repo, tag, commit or tag-created" >&2; exit 2
fi

if [ "$apply" -eq 0 ]; then
  echo "Dry run: nothing is written. Use --apply to do this."
  echo "  engine:  $repo, tag $tag, commit $commit"
  echo "  source:  ${engine_dir:-a shallow clone of the tag in a temp directory}"
  echo "  target:  $target ($([ -e "$target" ] && echo "exists, --apply would refuse" || echo "does not exist"))"
  echo "  steps:   git archive the commit into the target; check the files give the same git tree id as the commit"
  echo "           (paths, modes, bytes); write the target's LABEL.md (pre-event, not judged); remove the target if the check fails"
  echo "  after:   nothing is staged, committed or pushed; you stage with: git add -f pre-event-engine"
  exit 0
fi

[ ! -e "$target" ] || { echo "FAIL: $target exists; this script never overwrites, remove it first" >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
trap 'exit 1' HUP INT TERM
if [ -z "$engine_dir" ]; then
  engine_dir=$tmp/engine
  git clone -q --depth 1 --branch "$tag" "$repo" "$engine_dir" || { echo "cannot clone tag $tag from $repo" >&2; exit 2; }
fi
[ "$(git -C "$engine_dir" rev-parse --verify -q "$commit^{commit}" || true)" = "$commit" ] ||
  { echo "the engine at $engine_dir does not have commit $commit" >&2; exit 2; }
want=$(git -C "$engine_dir" rev-parse "$commit^{tree}")

mkdir "$target"
git -C "$engine_dir" archive --format=tar "$commit" | tar -x -C "$target"
# A throwaway repo: add -f takes every file, including dist/ that the engine's own .gitignore lists.
git init -q --bare "$tmp/check.git"
got=$(cd "$target" && git -c core.autocrlf=false -c core.safecrlf=false -c core.fileMode=true --git-dir="$tmp/check.git" --work-tree=. add -f -A . &&
  git --git-dir="$tmp/check.git" write-tree) || got=
if [ "$got" != "$want" ]; then
  rm -rf -- "${target:?}"
  echo "FAIL: the copy is not byte-identical to commit $commit (tree ${got:-unknown}, wanted $want); removed $target" >&2
  exit 1
fi

cat > "$target/LABEL.md" <<EOF
# pre-event, not judged

This folder is a byte-for-byte copy of the Torpor engine (${repo%.git}) at tag $tag, commit $commit.
The tag was created $created, before the Meta Global AI Developer Hackathon started.
It is a declared open-source base and is not part of the work to be judged.

scripts/vendor-engine.sh made the copy and checked it against the commit (git tree $want).
EOF

echo "ok: $target is byte-identical to commit $commit (git tree $want), plus LABEL.md"
echo "Nothing is staged, committed or pushed. Stage it with: git add -f pre-event-engine"
echo "(plain git add skips dist/, because the engine's own .gitignore lists it)"
