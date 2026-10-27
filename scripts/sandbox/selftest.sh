#!/usr/bin/env bash
# Selftest for muse-sandbox.sh (checks 1-4): no key and no internet needed. Never part of the gate or CI.
#   MUSE_BIN=/path/to/muse scripts/sandbox/selftest.sh      (without MUSE_BIN: `muse` from PATH)
# Needs bwrap, node >= 20, openssl, curl, and (for 1.6) xdpyinfo with a working DISPLAY=:0 on the host.
# Prints PASS/FAIL per assert, exits 0 only if all pass. Its files stay in a new muse-sandbox-selftest.* folder under /tmp.
set -u
export LC_ALL=C
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
SB=$HERE/muse-sandbox.sh
PASS=0 FAIL=0
pass() { PASS=$((PASS + 1)); echo "PASS $1"; }
fail() { FAIL=$((FAIL + 1)); echo "FAIL $1${2:+ [$2]}"; }
chk() { local r=$?; if [ "$r" -eq 0 ]; then pass "$1"; else fail "$1" "${2:-}"; fi; } # after a condition: chk "DESC" ["$detail"] (plain variables only: $(...) in the arguments would reset $?)
in_log() { grep -qE -- "$2" "$1"; }                                         # in_log FILE REGEX

T=$(mktemp -d "${TMPDIR:-/tmp}/muse-sandbox-selftest.XXXXXX") || exit 2
REAL=$(getent passwd "$(id -u)" | cut -d: -f6)
MUSE=${MUSE_BIN:-$(command -v muse || true)}
MUSE_DIR='' RO=()
if [ -n "$MUSE" ]; then MUSE_DIR=$(dirname "$(readlink -f "$MUSE")"); RO=(--ro "$MUSE_DIR"); fi
SERVER='' CANARY=''
trap '[ -z "$SERVER" ] || kill "$SERVER" 2>/dev/null; [ -z "$CANARY" ] || kill "$CANARY" 2>/dev/null' EXIT
sb() { local d=$1; shift; timeout 120 "$SB" --run-dir "$d" ${RO[@]+"${RO[@]}"} "$@" </dev/null; }

# throwaway servers on the host: P1 HTTP, P2 HTTPS (self-signed), P3 HTTP that must stay unreachable
openssl req -x509 -newkey rsa:2048 -nodes -keyout "$T/key.pem" -out "$T/cert.pem" -subj /CN=127.0.0.1 -days 1 \
  -addext subjectAltName=IP:127.0.0.1 >/dev/null 2>&1 || { echo "FAIL setup: openssl made no certificate"; exit 1; }
node "$HERE/selftest-servers.mjs" "$T/cert.pem" "$T/key.pem" >"$T/ports" 2>"$T/server.log" &
SERVER=$!
for _ in $(seq 50); do [ -s "$T/ports" ] && break; sleep 0.1; done
read -r P1 P2 P3 <"$T/ports" || { echo "FAIL setup: the test servers did not start"; exit 1; }
sleep 600 &
CANARY=$!

echo "== 1 isolation (--net none)"
R1=$T/r1; mkdir "$R1"
[ -z "$(sb "$R1" -- ls -A /run/user)" ]; chk "1.1 /run/user is empty inside"
[ "$(sb "$R1" -- ls -A /run)" = user ]; chk "1.2 /run holds only the empty user dir inside (no libvirt, dbus or docker sockets)"
expect=$'.local\n.local/share\n.local/share/muse'
case $MUSE_DIR in "$REAL"/*)
  p=${MUSE_DIR#"$REAL"/} acc=''
  while [ -n "$p" ]; do
    acc=${acc:+$acc/}${p%%/*}; expect+=$'\n'$acc
    case $p in */*) p=${p#*/} ;; *) p='' ;; esac
  done ;;
esac
extra=$(comm -13 <(sort <<<"$expect") <(sb "$R1" -- find "$REAL" -mindepth 1 -not -path "${MUSE_DIR:-/none}/*" \
  -not -path "$REAL/.local/share/muse/*" -printf '%P\n' 2>/dev/null | sort) | head -n 5 | tr '\n' ' ')
[ -z "$extra" ]; chk "1.3 the real home shows only the bound paths" "$extra"
M=.muse-sbx-selftest-$$
sb "$R1" -- touch "/tmp/$M" "$REAL/$M" "/var/tmp/$M" "/etc/$M" "$T/$M" "$R1/$M" >/dev/null 2>&1
leaks=''
for f in "/tmp/$M" "$REAL/$M" "/var/tmp/$M" "/etc/$M" "$T/$M"; do [ ! -e "$f" ] || leaks+=" $f"; done
[ -z "$leaks" ]; chk "1.4 writes to /tmp, the real home, /var/tmp, /etc and next to the run dir never reach the host" "$leaks"
[ -e "$R1/$M" ]; chk "1.5 control: a write inside the run dir does reach the host"
if DISPLAY=:0 xdpyinfo >/dev/null 2>&1; then
  sb "$R1" -- env DISPLAY=:0 xdpyinfo >/dev/null 2>&1; rc=$?
  [ "$rc" -eq 1 ]; chk "1.6 DISPLAY=:0 xdpyinfo works on the host and fails inside (rc=$rc, want 1)"
else
  fail "1.6 DISPLAY=:0 xdpyinfo" "NOT VERIFIED: it does not work on the host, so there is no control"
fi
sb "$R1" -- bash -c "exec 3<>/dev/tcp/127.0.0.1/$P1" >/dev/null 2>&1; rc=$?
[ "$rc" -eq 1 ]; chk "1.7 the host's loopback server is unreachable inside (rc=$rc, want 1)"
[ -z "$(sb "$R1" -- getent hosts example.com)" ]; chk "1.8 no DNS inside"
sb "$R1" -- kill -0 "$CANARY" >/dev/null 2>&1; rc=$?
kill -0 "$CANARY" && [ "$rc" -eq 1 ]; chk "1.9 a host process (alive: control) cannot be signalled from inside (rc=$rc, want 1)"
envout=$(FOO=bar DIR=/x CHDIR=/x DRAIN=5 META_API_KEY=not-a-key TORPOR_SELFTEST=1 sb "$R1" -- env)
grep -qx TORPOR_SELFTEST=1 <<<"$envout" && grep -qx "HOME=$R1/home" <<<"$envout" && ! grep -qE '^(FOO|DIR|CHDIR|DRAIN|META_API_KEY)=' <<<"$envout"
chk "1.10 env cleared: FOO, META_API_KEY (with --net none) and a caller's DIR, CHDIR, DRAIN gone, TORPOR_* and HOME kept"
bad=''
for r in "$REAL" / relative "$T/missing"; do "$SB" --run-dir "$r" -- true >/dev/null 2>&1; [ $? -eq 125 ] || bad+=" $r"; done
PATH=/nonexistent "$BASH" "$SB" --run-dir "$R1" -- true >/dev/null 2>&1; [ $? -eq 125 ] || bad+=" no-bwrap"
sb "$R1" -- sh -c 'exit 7' >/dev/null 2>&1; [ $? -eq 7 ] || bad+=" exit-code"
[ -z "$bad" ]; chk "1.11 refuses home, /, relative and missing run dirs and a missing bwrap with 125; returns the command's code" "$bad"

echo "== 2 allowlist (--net meta)"
R2=$T/r2; mkdir "$R2"; L=$R2/net/proxy.log
A=(--net meta --allow "127.0.0.1:$P1" --allow api.meta.ai:443)
out=$(sb "$R2" "${A[@]}" -- curl -sS --max-time 10 "http://127.0.0.1:$P1/page" 2>&1)
[ "$out" = TOKEN-HTTP ]; chk "2.1 a request to the allowed local server gets its page" "$out"
out=$(sb "$R2" "${A[@]}" -- curl -sS --max-time 10 https://example.com/ 2>&1); rc=$?
[ "$rc" -ne 0 ] && ! grep -q 'Example Domain' <<<"$out"; chk "2.2 https://example.com is refused (rc=$rc)"
out=$(sb "$R2" "${A[@]}" -- curl -sS --max-time 5 "http://127.0.0.1:$P3/" 2>&1); rc=$?
[ "$rc" -ne 0 ] && ! grep -q TOKEN-OTHER "$T/server.log"; chk "2.3 an unlisted local server stays unreachable (rc=$rc)"
out=$(sb "$R2" "${A[@]}" -- getent hosts api.meta.ai)
[[ $out == "127.0.1.1 "* ]]; chk "2.4 /etc/hosts maps api.meta.ai to its own loopback address" "$out"
sb "$R2" "${A[@]}" -- bash -c 'exec 3<>/dev/tcp/127.0.1.1/443' >/dev/null 2>&1; rc=$?
[ "$rc" -eq 0 ]; chk "2.5 a forwarder listens on port 443 at that address (rc=$rc)"
in_log "$L" " 127[.]0[.]0[.]1:$P1 allowed up=[0-9]+ down=[0-9]+ via=fwd" && in_log "$L" ' api[.]meta[.]ai:443 allowed ' \
  && in_log "$L" ' example[.]com:443 denied up=0 down=0 via=proxy'
chk "2.6 proxy.log has the allowed page, the allowed name and the refused host"
[ -s "$L" ] && ! grep -vqE '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z [^ ]+:[0-9]+ (allowed|denied) up=[0-9]+ down=[0-9]+' "$L"
chk "2.7 every proxy.log line is: UTC time, host:port, allowed or denied, bytes each way"
[ -z "$(sb "$R2" "${A[@]}" -- ls -A "$R2/net")" ]; chk "2.8 the proxy log and socket are hidden from inside"
[ ! -S "$R2/net/proxy.sock" ]; chk "2.9 the proxy is gone after the command exits"
envout=$(META_API_KEY=not-a-key sb "$R2" "${A[@]}" -- env)
grep -qx META_API_KEY=not-a-key <<<"$envout"; chk "2.10 META_API_KEY reaches the command with --net meta"
R2B=$T/r2b; mkdir "$R2B"
sb "$R2B" --net meta -- bash -c "exec 3<>/dev/tcp/127.0.0.1/$P1" >/dev/null 2>&1; rc=$?
sb "$R2B" --net meta -- bash -c 'exec 3<>/dev/tcp/127.0.1.1/443' >/dev/null 2>&1; rc2=$?
[ "$rc" -eq 1 ] && [ "$rc2" -eq 0 ]; chk "2.11 default allowlist is api.meta.ai:443 only (local server rc=$rc want 1, api.meta.ai rc=$rc2 want 0)"

R2C=$T/r2c; mkdir "$R2C"
cat >"$R2C/late.sh" <<'EOF'
#!/bin/sh
# runs detached, after the command that started it has exited: waits, then makes one request
sleep 2
curl -sS --max-time 5 "http://127.0.0.1:$2/late" >"$1/late.txt" 2>&1
EOF
cat >"$R2C/start.sh" <<'EOF'
#!/bin/sh
setsid sh "$1/late.sh" "$1" "$2" </dev/null >/dev/null 2>&1 &
EOF
sb "$R2C" --net meta --allow "127.0.0.1:$P1" -- sh "$R2C/start.sh" "$R2C" "$P1" >/dev/null 2>&1
[ "$(cat "$R2C/late.txt" 2>/dev/null)" = TOKEN-HTTP ]; chk "2.12 a detached process that outlives the command still gets through (the launcher waits for it)"

echo "== 3 hook path (--net meta, ECHO Muse run, a hook makes one HTTPS request)"
if [ -z "$MUSE" ]; then fail "3 and 4 need Muse" "NOT VERIFIED: set MUSE_BIN or put muse on PATH"; else
R3=$T/r3; H3=$R3/home; mkdir -p "$H3/.config/muse" # H3 keeps a literal home path out of the source
cp "$T/cert.pem" "$R3/cert.pem"
cat >"$R3/hook.sh" <<EOF
#!/bin/sh
d=\$(dirname "\$0")
env | cut -d= -f1 | sort >"\$d/hook-env.txt"
curl -sS --max-time 15 --cacert "\$d/cert.pem" https://127.0.0.1:$P2/hook >"\$d/hook-out.txt" 2>"\$d/hook-err.txt"
echo \$? >"\$d/hook-rc.txt"
cat >/dev/null
EOF
cat >"$H3/.config/muse/settings.json" <<EOF
{"schema_version":1,"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"sh $R3/hook.sh","timeout":20}]}]}}
EOF
ECHO=(exec --provider echo --trust-workspace --disable-reminders --contributor-mode off --approval-judge off --disable-web-tools)
out=$(sb "$R3" --net meta --allow "127.0.0.1:$P2" -- "$MUSE" "${ECHO[@]}" ping 2>/dev/null); rc=$?
[ "$rc" -eq 0 ] && grep -qx 'echo: ping' <<<"$out"; chk "3.1 the ECHO run exits 0 and answers (rc=$rc)"
[ "$(cat "$R3/hook-out.txt")" = TOKEN-HTTPS ] && [ "$(cat "$R3/hook-rc.txt")" = 0 ]; chk "3.2 the hook got the page over HTTPS"
grep -qx 'TOKEN-HTTPS /hook' "$T/server.log"; chk "3.3 the host server saw the hook's request"
in_log "$R3/net/proxy.log" " 127[.]0[.]0[.]1:$P2 allowed .*via=fwd"; chk "3.4 proxy.log shows it came through the forwarder, not the proxy env"
echo "info: env names that reached the hook: $(tr '\n' ' ' <"$R3/hook-env.txt")"

echo "== 4 Muse works inside (--net none, no key)"
R4=$T/r4; mkdir "$R4"
out=$(sb "$R4" -- "$MUSE" --version 2>&1); rc=$?
[ "$rc" -eq 0 ] && grep -qE '^Muse Code [0-9]' <<<"$out"; chk "4.1 --version works (rc=$rc): $out"
out=$(sb "$R4" -- "$MUSE" "${ECHO[@]}" ping 2>/dev/null); rc=$?
[ "$rc" -eq 0 ] && grep -qx 'echo: ping' <<<"$out"; chk "4.2 one ECHO exec works (rc=$rc)"
fi

echo "selftest: $PASS passed, $FAIL failed (files kept in $T)"
[ "$FAIL" -eq 0 ]
