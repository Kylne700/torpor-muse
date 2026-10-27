#!/bin/sh
# Clean-room check of the README install block. release.yml runs it on a bare Debian container and on a macOS runner.
#   cleanroom.sh <dir>      <dir> holds the release assets (torpor-muse-<platform>, four of them) and SHA256SUMS
# 1. Takes the one sh block between <!-- INSTALL:start --> and <!-- INSTALL:end --> in README.md and checks it against
#    engine.lock (engine commit and the four sha256) and Cargo.toml (the version in the release URL).
# 2. Runs that block as written, in an empty HOME with a bare environment. Only its release base URL is changed, to
#    <dir>, so the README cannot drift from what is tested. The pinned engine still comes from GitHub.
# 3. Does the offline first-run steps (version, init, doctor, uninstall) and asserts what each prints without Muse Code
#    and without a key. What needs Muse Code or a Meta key is printed at the end as keyed, not faked.
# Exit 0 = every assertion held, 1 = one failed, 2 = bad usage.
set -eu
export LC_ALL=C
cd "$(dirname "$0")/.."

fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }
lock() { awk -v k="$1" '$1 == k { print $2; exit }' engine.lock; }

[ $# -eq 1 ] && [ -d "$1" ] || { echo "usage: cleanroom.sh <release asset dir>" >&2; exit 2; }
art=$(cd "$1" && pwd -P)
tmp=$(cd "$(mktemp -d)" && pwd -P) # physical path: on macOS /var is a link and init prints the resolved engine path
trap 'rm -rf "$tmp"' EXIT
trap 'exit 1' HUP INT TERM

ver=$(awk -F'"' '$1 == "version = " { print $2; exit }' Cargo.toml)
[ -n "$ver" ] || fail "no version line in Cargo.toml"
if [ "${GITHUB_REF_TYPE:-}" = tag ] && [ "${GITHUB_REF_NAME:-}" != "v$ver" ]; then
  fail "tag ${GITHUB_REF_NAME:-} is not v$ver (Cargo.toml)"
fi

# ---- the assets: four binaries, one SHA256SUMS that covers them
sums() { if command -v sha256sum >/dev/null 2>&1; then sha256sum -c "$1"; else shasum -a 256 -c "$1"; fi; }
for p in linux-x64 linux-arm64 darwin-x64 darwin-arm64; do
  [ -f "$art/torpor-muse-$p" ] || fail "no torpor-muse-$p in $art"
done
[ "$(ls "$art" | wc -l | tr -d " ")" -eq 5 ] || fail "$art should hold the four binaries and SHA256SUMS, nothing else"
(cd "$art" && sums SHA256SUMS >/dev/null) || fail "SHA256SUMS does not match the assets"
ok "four assets and SHA256SUMS agree"

# ---- the README block, and what it must agree with
[ "$(grep -c '^<!-- INSTALL:start -->$' README.md)" -eq 1 ] || fail "README.md needs exactly one INSTALL:start marker"
[ "$(grep -c '^<!-- INSTALL:end -->$' README.md)" -eq 1 ] || fail "README.md needs exactly one INSTALL:end marker"
awk '/^<!-- INSTALL:start -->$/ { s = 1; next } /^<!-- INSTALL:end -->$/ { s = 0 } s' README.md > "$tmp/section.md"
[ "$(grep -c '^```' "$tmp/section.md")" -eq 2 ] && [ "$(grep -c '^```sh$' "$tmp/section.md")" -eq 1 ] ||
  fail "the INSTALL section needs exactly one fenced sh block"
awk '/^```sh$/ { f = 1; next } /^```$/ { f = 0 } f' "$tmp/section.md" > "$tmp/block.sh"
[ -s "$tmp/block.sh" ] || fail "the INSTALL block is empty"

[ "$(grep -c '^rel=' "$tmp/block.sh")" -eq 1 ] || fail "the block needs exactly one rel= line"
grep -Fxq "rel=https://github.com/Kylne700/torpor-muse/releases/download/v$ver" "$tmp/block.sh" ||
  fail "the block's rel= line is not the v$ver release of Kylne700/torpor-muse (Cargo.toml version)"
grep -Fxq "eng=https://raw.githubusercontent.com/Kylne700/torpor/$(lock commit)/dist" "$tmp/block.sh" ||
  fail "the block's eng= line is not the engine commit $(lock commit) of engine.lock"
awk '$1 == "asset" { print $2, $3 }' engine.lock > "$tmp/assets"
[ "$(wc -l < "$tmp/assets" | tr -d " ")" -eq 4 ] || fail "engine.lock should list four engine assets"
while read -r p h; do
  grep -Fq "p=$p e=$h ;;" "$tmp/block.sh" || fail "the block lacks the engine.lock sha256 of $p"
done < "$tmp/assets"
ok "the block names release v$ver, engine commit $(lock commit | cut -c1-7) and the four engine.lock sha256"

# ---- run the block as written: empty HOME, bare environment, release URL -> the artifacts of this run
sed "s|^rel=.*|rel=file://$art|" "$tmp/block.sh" > "$tmp/block-run.sh"
[ "$(grep -Fxc "rel=file://$art" "$tmp/block-run.sh")" -eq 1 ] || fail "could not point the block's rel= line at $art"
H=$tmp/home
mkdir "$H"
echo "--- README install block, as written (empty HOME, rel= -> file://$art)"
P=$(env -i HOME="$H" PATH=/usr/bin:/bin:/usr/sbin:/sbin sh -c '. "$1" >&2 && printf %s "$PATH"' sh "$tmp/block-run.sh") ||
  fail "the README install block failed"
case $P in "$H/.local/bin:"*) ok "the block put \$HOME/.local/bin first on PATH" ;; *) fail "PATH after the block is $P" ;; esac

# step <title> <expected exit code> <command...>: runs it in the bare environment (HOME and the PATH the block made);
# stdout goes to $tmp/out, stderr to $tmp/err; only() asserts the stdout is exactly one text.
step() {
  title=$1 want=$2
  shift 2
  echo "--- $title"
  rc=0
  env -i HOME="$H" PATH="$P" "$@" > "$tmp/out" 2> "$tmp/err" || rc=$?
  sed 's/^/  | /' "$tmp/out"
  sed 's/^/  ! /' "$tmp/err"
  [ "$rc" -eq "$want" ] || fail "'$title' exited $rc, expected $want"
}
only() { [ "$(cat "$tmp/out")" = "$1" ] || fail "'$title' printed '$(cat "$tmp/out")', expected '$1'"; }
snap() { (cd "$H" && find . | sort); }

step "which torpor-muse" 0 sh -c 'command -v torpor-muse'
only "$H/.local/bin/torpor-muse"
step "which torpor" 0 sh -c 'command -v torpor'
only "$H/.local/bin/torpor"

# ---- the offline first run: no Muse Code, no key
step "torpor-muse version" 0 torpor-muse version
only "torpor-muse $ver"
step "torpor version (the pinned engine)" 0 torpor version
only "torpor $(lock version)"

snap > "$tmp/before"
s=$H/.config/muse/settings.json
step "torpor-muse init" 0 torpor-muse init
[ "$(sed -n 1p "$tmp/out")" = "torpor-muse hooks installed in $s" ] || fail "init line 1 is not 'torpor-muse hooks installed in $s'"
[ "$(sed -n 2p "$tmp/out")" = "store $H/.torpor, engine $H/.local/bin/torpor; start a new Muse session to use them" ] ||
  fail "init line 2 does not name the store, the engine on PATH and the new session"
[ "$(wc -l < "$tmp/out" | tr -d " ")" -eq 2 ] || fail "init printed more than two lines"
for e in SessionStart UserPromptSubmit Stop; do
  grep -Fq "$H/.local/bin/torpor-muse hook $e\"" "$s" || fail "$s has no $e hook that runs the installed torpor-muse"
done
step "torpor-muse init (again)" 0 torpor-muse init
only "torpor-muse hooks are already installed in $s"

# Without a Muse session doctor cannot prove the gate live: exit 1 with that one FIX and nothing else to fix.
step "torpor-muse doctor" 1 torpor-muse doctor
grep -q '^ok   vault: ' "$tmp/out" || fail "doctor has no 'ok   vault:' line"
grep -q '^ok   probe: ' "$tmp/out" || fail "doctor has no 'ok   probe:' line"
[ "$(grep -c '^FIX' "$tmp/out")" -eq 1 ] || fail "doctor should report exactly one FIX"
grep -q '^FIX  gate not proven live' "$tmp/out" || fail "doctor's one FIX is not 'gate not proven live'"
grep -Fxq 'doctor: 1 to fix' "$tmp/out" || fail "doctor's last line is not 'doctor: 1 to fix'"

step "torpor-muse uninstall" 0 torpor-muse uninstall
only "torpor-muse hooks removed; $s was created by init and is deleted"
[ ! -e "$s" ] || fail "uninstall left $s"
step "torpor-muse uninstall (again)" 0 torpor-muse uninstall
only "nothing to do: $s does not exist"

# HOME as it was? Not quite: the directories init made and the engine-ok cache (engine::verify) stay. Pinned, so a
# third leftover fails here.
snap > "$tmp/after"
[ -z "$(comm -23 "$tmp/before" "$tmp/after")" ] || fail "uninstall removed something that was there before init"
comm -13 "$tmp/before" "$tmp/after" > "$tmp/left"
printf '%s\n' ./.config ./.config/muse ./.torpor ./.torpor/muse ./.torpor/muse/engine-ok | sort > "$tmp/known"
[ -z "$(comm -23 "$tmp/left" "$tmp/known")" ] || fail "uninstall left more than the known leftovers: $(comm -23 "$tmp/left" "$tmp/known")"
[ ! -s "$tmp/left" ] || echo "KNOWN leftover after init + uninstall (open problem, HOME is not exactly as it was): $(tr '\n' ' ' < "$tmp/left")"
ok "first run: version, init, doctor, uninstall print what is asserted above"

cat <<'EOF'
keyed (the lead runs these on Nov 1; they need Muse Code or a Meta key, so they are not run or faked here):
  - Muse Code itself: starting a new session after `torpor-muse init`
  - the SessionStart, UserPromptSubmit and Stop hooks firing in that session and driving the engine
  - `torpor-muse doctor` clean after one prompt (the 'gate not proven live' FIX is gone)
  - `torpor-muse suggest "<what works>"` (asks Muse Spark, with the Meta key)
cleanroom: OK
EOF
