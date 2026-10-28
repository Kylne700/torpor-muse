#!/bin/sh
# Tests for scripts/new-work.sh and scripts/verify-engine.sh, run on a temp clone of this
# repo (its committed state, so commit before you run). Needs git and the network (GitHub).
#   sh scripts/test/run.sh
# Optional environment:
#   ENGINE_DIR  a clone of the engine that has the pinned commit (else one is cloned)
#   ASSET       a downloaded dist/linux-x64/torpor (else the file from the engine clone is used)
#   FORBID_DIR  a directory whose files new-work.sh must refuse (else a made-up one is used)
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
trap 'exit 1' HUP INT TERM
pass=0
failed=0
lockval() { awk -v k="$1" '$1 == k { print $2; exit }' "$root/engine.lock"; }
repo=$(lockval repo); tag=$(lockval tag); commit=$(lockval commit)

# t <name> <ok|fail> <text the output must contain, or ""> <command...>
t() {
  name=$1; want=$2; text=$3; shift 3
  if "$@" > "$tmp/out" 2>&1; then got=ok; else got=fail; fi
  if [ "$got" = "$want" ] && { [ -z "$text" ] || grep -q -F -- "$text" "$tmp/out"; }; then
    pass=$((pass + 1)); echo "PASS $name"
  else
    failed=$((failed + 1)); echo "FAIL $name (it ended $got, wanted $want${text:+ with output containing: $text})"
    sed 's/^/    | /' "$tmp/out" | head -n 15
  fi
}

c=$tmp/c
git clone -q "$root" "$c"
git -C "$c" config user.name test
git -C "$c" config user.email ci

engine=${ENGINE_DIR:-}
[ -n "$engine" ] || { engine=$tmp/engine; git clone -q --depth 1 --branch "$tag" "$repo" "$engine"; }
asset=${ASSET:-}
[ -n "$asset" ] || { asset=$engine/dist/linux-x64/torpor; echo "note: ASSET not set, using the file from the engine clone, not a release download"; }
forbid=${FORBID_DIR:-}
[ -n "$forbid" ] || { forbid=$tmp/forbid; mkdir "$forbid"; echo "a pre-event spec file" > "$forbid/spec.txt"; echo "note: FORBID_DIR not set, using a made-up directory"; }
planted=$(find "$forbid" -type f | LC_ALL=C sort | head -n 1)

nw() { sh "$c/scripts/new-work.sh" --engine "$engine" "$@"; }
ve() { sh "$c/scripts/verify-engine.sh" "$@"; }
fromdir() { d=$1; shift; (cd "$d" && "$@"); }

echo "== new-work.sh"
t "passes on the branch (engine from a local clone)" ok "new-work: ok" nw
t "passes with the engine tree fetched from GitHub" ok "new-work: ok" sh "$c/scripts/new-work.sh"
t "shows the 16:05 commit as README + licences" ok "README + licences only: LICENSE-APACHE LICENSE-MIT README.md" nw
sed 's/Built during the hackathon/Built during the hackathon, edited/' "$c/README.md" > "$tmp/r" && cp "$tmp/r" "$c/README.md"
t "fails when the README block is edited" fail "the README block differs from NEW-VS-OLD.md" nw
git -C "$c" checkout -q -- README.md
grep -v 'NEW-VS-OLD:end' "$c/README.md" > "$tmp/r" && cp "$tmp/r" "$c/README.md"
t "fails when a README marker is missing" fail "needs exactly one" nw
git -C "$c" checkout -q -- README.md
git -C "$engine" show "$commit:Cargo.toml" > "$c/planted-cargo.toml"
git -C "$c" add planted-cargo.toml
t "fails on a planted file that equals an engine file" fail "planted-cargo.toml is byte-identical to engine:Cargo.toml" nw
git -C "$c" rm -q -f --cached planted-cargo.toml; rm "$c/planted-cargo.toml"
cp "$planted" "$c/planted-forbidden"
git -C "$c" add planted-forbidden
t "passes a planted --forbid file when --forbid is not given" ok "new-work: ok" nw
t "fails on a planted copy of a --forbid file" fail "planted-forbidden is byte-identical to" nw --forbid "$forbid"
t "reads a relative --forbid path from the caller's directory" fail "planted-forbidden is byte-identical to" \
  fromdir "$(dirname "$forbid")" sh "$c/scripts/new-work.sh" --engine "$engine" --forbid "$(basename "$forbid")"
git -C "$c" rm -q -f --cached planted-forbidden; rm "$c/planted-forbidden"
echo "code" > "$c/early.rs"
git -C "$c" add early.rs
GIT_AUTHOR_DATE=2026-10-26T16:30:00Z GIT_COMMITTER_DATE=2026-10-26T16:30:00Z git -C "$c" commit -q -m "code before the gate"
t "fails on code committed before the gate" fail "CODE BEFORE THE GATE" nw
git -C "$c" reset -q --hard HEAD~1
git clone -q --depth 1 "file://$root" "$tmp/shallow"
t "fails on a shallow clone" fail "shallow clone" sh "$tmp/shallow/scripts/new-work.sh" --engine "$engine"

echo "== verify-engine.sh"
t "accepts the locked linux-x64 binary" ok "ok:" ve "$asset"
t "accepts it when the platform is named" ok "is the locked linux-x64 binary" ve "$asset" linux-x64
cp "$asset" "$tmp/changed"
b=$(dd if="$asset" bs=1 skip=100 count=1 2>/dev/null | od -An -tu1 | tr -d ' ')
printf '%b' "\\0$(printf '%03o' $(((b + 1) % 256)))" | dd of="$tmp/changed" bs=1 seek=100 count=1 conv=notrunc 2>/dev/null
t "the changed copy differs from the asset in exactly one byte" ok "" test "$(cmp -l "$asset" "$tmp/changed" | wc -l | tr -d ' ')" = 1
t "rejects a 1-byte change" fail "matches no locked asset" ve "$tmp/changed"
t "rejects the right file under another platform" fail "matches no locked asset for darwin-arm64" ve "$asset" darwin-arm64
t "an unknown platform name gets the list of locked ones" fail "locked platforms: darwin-arm64" ve "$asset" linux-amd64
t "--tag: the remote tag resolves to the locked commit" ok "resolves to the locked commit" ve --tag
sed "s/^commit .*/commit $(lockval base-commit)/" "$c/engine.lock" > "$tmp/l" && cp "$tmp/l" "$c/engine.lock"
t "--tag fails when the lock pins another commit (a moved tag)" fail "now resolves to" ve --tag
git -C "$c" checkout -q -- engine.lock

echo "$pass passed, $failed failed"
[ "$failed" -eq 0 ]
