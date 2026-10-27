#!/bin/sh
# Checks the engine pin in engine.lock.
#   verify-engine.sh <file> [<platform>]   the sha256 of <file> must equal a locked asset (that platform's, if given)
#   verify-engine.sh --tag                 the remote tag must still resolve to the locked commit
# Exit 0 = ok, 1 = check failed, 2 = bad usage or bad lock.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
lock=$root/engine.lock

fail() { echo "FAIL: $*" >&2; exit 1; }
usage() { echo "usage: verify-engine.sh <file> [<platform>] | --tag" >&2; exit 2; }
lockval() { awk -v k="$1" '$1 == k { print $2; exit }' "$lock"; }
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1
}

[ -f "$lock" ] || { echo "no engine.lock in $root" >&2; exit 2; }
[ $# -ge 1 ] || usage

case $1 in
  --tag)
    repo=$(lockval repo); tag=$(lockval tag); want=$(lockval commit)
    if [ -z "$repo" ] || [ -z "$tag" ] || [ -z "$want" ]; then echo "engine.lock lacks repo, tag or commit" >&2; exit 2; fi
    out=$(git ls-remote "$repo" "refs/tags/$tag" "refs/tags/$tag^{}") || fail "git ls-remote $repo failed"
    # An annotated tag lists the tag object and, as <tag>^{}, the commit it points to; a lightweight tag lists the commit.
    got=$(printf '%s\n' "$out" | awk -v t="refs/tags/$tag" '
      $2 == (t "^{}") { peeled = $1 }
      $2 == t { plain = $1 }
      END { print (peeled != "" ? peeled : plain) }')
    [ -n "$got" ] || fail "tag $tag not found in $repo"
    [ "$got" = "$want" ] || fail "tag $tag now resolves to $got, engine.lock pins $want"
    echo "ok: tag $tag resolves to the locked commit $want"
    ;;
  -*) usage ;;
  *)
    file=$1; platform=${2:-}
    [ -f "$file" ] || { echo "no such file: $file" >&2; exit 2; }
    got=$(sha256 "$file")
    hit=$(awk -v h="$got" -v p="$platform" '$1 == "asset" && $3 == h && (p == "" || $2 == p) { print $2; exit }' "$lock")
    names=$(awk '$1 == "asset" { printf "%s%s", sep, $2; sep = " " }' "$lock")
    [ -n "$hit" ] || fail "sha256 $got of $file matches no locked asset${platform:+ for $platform (locked platforms: $names)}"
    echo "ok: $file is the locked $hit binary (sha256 $got)"
    ;;
esac
