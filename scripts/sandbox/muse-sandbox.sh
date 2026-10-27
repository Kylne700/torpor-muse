#!/usr/bin/env bash
# muse-sandbox.sh: run CMD in a bubblewrap sandbox with no network, or with only an allowlist (see README.md).
#   muse-sandbox.sh --run-dir DIR [--ro PATH]... [--net none|meta] [--allow HOST:PORT]... [--chdir DIR2] [--drain SECS] -- CMD [ARGS...]
IFS=$' \t\n'
for f in $(compgen -A function); do unset -f "$f"; done # functions imported from the caller's env would reach the sandbox
set -u

die() { echo "muse-sandbox: $*" >&2; exit 125; }

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P) || die "cannot find my own folder"
RUN_DIR='' NET=none CHDIR='' DRAIN=90
RO=() ALLOW=()
while [ $# -gt 0 ]; do
  case $1 in
    -h | --help) echo "usage: muse-sandbox.sh --run-dir DIR [--ro PATH]... [--net none|meta] [--allow HOST:PORT]... [--chdir DIR2] [--drain SECS] -- CMD [ARGS...]"; exit 0 ;;
    --run-dir | --ro | --net | --allow | --chdir | --drain)
      [ $# -ge 2 ] || die "$1 needs a value"
      case $1 in
        --run-dir) RUN_DIR=$2 ;;
        --ro) RO+=("$2") ;;
        --net) NET=$2 ;;
        --allow) ALLOW+=("$2") ;;
        --chdir) CHDIR=$2 ;;
        --drain) DRAIN=$2 ;;
      esac
      shift 2 ;;
    --) shift; break ;;
    *) die "unknown option: $1 (the command goes after --)" ;;
  esac
done
[ $# -gt 0 ] || die "no command given (it goes after --)"
case $NET in none | meta) ;; *) die "--net must be none or meta" ;; esac
[ "$NET" = meta ] || [ ${#ALLOW[@]} -eq 0 ] || die "--allow needs --net meta"
[[ $DRAIN =~ ^[0-9]{1,5}$ ]] || die "--drain wants a number of seconds"
BWRAP=$(command -v bwrap) || die "bwrap is not installed (package bubblewrap)"

# --- the run dir: absolute, existing, and never the home, a parent of it, or /
[ -n "$RUN_DIR" ] || die "--run-dir is required"
case $RUN_DIR in /*) ;; *) die "--run-dir must be an absolute path: $RUN_DIR" ;; esac
[ -d "$RUN_DIR" ] || die "--run-dir is not an existing directory: $RUN_DIR"
DIR=$(cd "$RUN_DIR" && pwd -P) || die "cannot enter --run-dir: $RUN_DIR"
[ "$DIR" != / ] || die "--run-dir must not be /"
REAL=$(getent passwd "$(id -u)" | cut -d: -f6)
[ -d "${REAL:-}" ] || REAL=${HOME:-}
[ -d "$REAL" ] || die "cannot find the real home directory"
REAL=$(cd "$REAL" && pwd -P)
for h in "$REAL" "${HOME:-$REAL}"; do
  h=$(cd "$h" 2>/dev/null && pwd -P) || continue
  case "$h/" in "$DIR/"*) die "--run-dir must be below the home directory, not equal to it or above it: $DIR" ;; esac
done
for p in ${RO[@]+"${RO[@]}"}; do
  case $p in /*) ;; *) die "--ro must be an absolute path: $p" ;; esac
  [ -e "$p" ] || die "--ro path does not exist: $p"
done
CHDIR=${CHDIR:-$DIR}
case $CHDIR in /*) ;; *) die "--chdir must be an absolute path: $CHDIR" ;; esac
mkdir -p "$DIR/home" "$DIR/tmp" "$DIR/muse-share" || die "cannot create folders in $DIR"
UID_=$(id -u) GID_=$(id -g)

# --- --net meta: work out which names get which loopback address, and start the host proxy
PROXY_PID=''
# shellcheck disable=SC2317 # runs from the EXIT trap
cleanup() { [ -z "$PROXY_PID" ] || { kill "$PROXY_PID" 2>/dev/null; wait "$PROXY_PID" 2>/dev/null; }; return 0; }
trap cleanup EXIT
if [ "$NET" = meta ]; then
  NODE=$(command -v node) || die "--net meta needs node (version 20 or newer)"
  "$NODE" -e 'process.exit(+process.versions.node.split(".")[0] >= 20 ? 0 : 1)' || die "--net meta needs node 20 or newer"
  [ $((${#DIR} + 15)) -le 107 ] || die "--run-dir path is too long for the unix socket in it: $DIR"
  [ ${#ALLOW[@]} -gt 0 ] || ALLOW=(api.meta.ai:443)
  SERVE_ALLOW=() FWD=() HOSTS_ADD=()
  declare -A ADDR_OF=()
  NEXT=1
  for a in "${ALLOW[@]}"; do
    [[ $a =~ ^(.+):([0-9]{1,5})$ ]] || die "--allow wants HOST:PORT, got: $a"
    h=${BASH_REMATCH[1],,} p=$((10#${BASH_REMATCH[2]}))
    h=${h#[} h=${h%]}
    ((p >= 1 && p <= 65535)) || die "bad port in --allow: $a"
    [[ $h =~ ^[a-z0-9:._-]+$ && $h != -* ]] || die "bad host in --allow: $a"
    [ "$h" != localhost ] || h=127.0.0.1
    if [[ $h == *:* ]]; then
      SERVE_ALLOW+=("[$h]:$p") # IPv6 literal: reachable only through HTTPS_PROXY clients
    elif [[ $h =~ ^127(\.[0-9]{1,3}){3}$ ]]; then
      [ "$h:$p" != 127.0.0.1:3128 ] || die "127.0.0.1:3128 is reserved for the proxy bridge"
      SERVE_ALLOW+=("$h:$p")
      FWD+=(--fwd "$h:$p=$h:$p") # same address inside the sandbox, so the client needs no proxy setting
    elif [[ $h =~ ^[0-9]+(\.[0-9]+){3}$ ]]; then
      SERVE_ALLOW+=("$h:$p") # other IPv4 literal: reachable only through HTTPS_PROXY clients
    else
      if [ -z "${ADDR_OF[$h]:-}" ]; then
        [ "$NEXT" -le 254 ] || die "too many --allow host names"
        ADDR_OF[$h]=127.0.1.$NEXT
        HOSTS_ADD+=("127.0.1.$NEXT $h")
        NEXT=$((NEXT + 1))
      fi
      SERVE_ALLOW+=("$h:$p")
      FWD+=(--fwd "${ADDR_OF[$h]}:$p=$h:$p") # /etc/hosts sends the name here, then the bytes go to the host proxy
    fi
  done
  mkdir -p "$DIR/net" || die "cannot create $DIR/net"
  { cat /etc/hosts; [ ${#HOSTS_ADD[@]} -eq 0 ] || printf '%s\n' "${HOSTS_ADD[@]}"; } >"$DIR/net/hosts" || die "cannot write $DIR/net/hosts"
  : >"$DIR/net/proxy.out"
  SERVE=("$NODE" "$HERE/netproxy.mjs" serve --sock "$DIR/net/proxy.sock" --log "$DIR/net/proxy.log" --parent "$$")
  for a in "${SERVE_ALLOW[@]}"; do SERVE+=(--allow "$a"); done
  env -i PATH=/usr/bin:/bin "${SERVE[@]}" >"$DIR/net/proxy.out" 2>>"$DIR/net/proxy.err" &
  PROXY_PID=$!
  for ((i = 0; i < 100; i++)); do
    [ -s "$DIR/net/proxy.out" ] && break
    kill -0 "$PROXY_PID" 2>/dev/null || break
    sleep 0.1
  done
  [ -s "$DIR/net/proxy.out" ] || die "the host proxy did not start: $(tail -n 1 "$DIR/net/proxy.err" 2>/dev/null)"
fi

# --- the environment inside: cleared (never through argv, so the key stays out of ps), then a fixed set
KEEP_KEY=''
[ "$NET" = meta ] && [ -n "${META_API_KEY:-}" ] && KEEP_KEY=1
# shellcheck disable=SC2163 # $v holds the name of the variable to un-export
for v in $(compgen -e); do
  case $v in
    TORPOR_*) ;;
    META_API_KEY) [ -n "$KEEP_KEY" ] || unset META_API_KEY ;;
    *) export -n "$v" 2>/dev/null ;; # un-export, not unset: a caller's DIR, CHDIR or DRAIN must not wipe this script's own
  esac
done
export HOME="$DIR/home" TMPDIR="$DIR/tmp" LANG=C.UTF-8 PATH=/usr/local/bin:/usr/bin:/bin MUSE_NO_AUTO_UPDATE=1 PWD="$CHDIR"
export XDG_CONFIG_HOME="$HOME/.config" XDG_DATA_HOME="$HOME/.local/share" XDG_STATE_HOME="$HOME/.local/state" XDG_CACHE_HOME="$HOME/.cache"
if [ "$NET" = meta ]; then
  # courtesy for clients that honour it (Muse, curl): their refused requests reach the proxy and get logged.
  # Hooks and the detached writer do not get these (Muse clears the hook env) and use the /etc/hosts path instead.
  export HTTPS_PROXY=http://127.0.0.1:3128 https_proxy=http://127.0.0.1:3128 NO_PROXY=127.0.0.1,localhost,::1 no_proxy=127.0.0.1,localhost,::1
fi

# --- the sandbox
BW=(--ro-bind / / --dev /dev --proc /proc --tmpfs /tmp --tmpfs "$REAL" --tmpfs /run --dir /run/user)
for p in ${RO[@]+"${RO[@]}"}; do BW+=(--ro-bind "$p" "$p"); done
BW+=(--bind "$DIR" "$DIR" --bind "$DIR/muse-share" "$REAL/.local/share/muse")
[ "$NET" != meta ] || BW+=(--tmpfs "$DIR/net" --ro-bind "$DIR/net/hosts" /etc/hosts) # hides the proxy log and socket from the model
BW+=(--unshare-ipc --new-session --die-with-parent --chdir "$CHDIR")

if [ "$NET" = none ]; then
  exec "$BWRAP" "${BW[@]}" --unshare-net --unshare-pid -- "$@"
fi

# Two layers. A single bwrap cannot bind port 443 in its network namespace even with CAP_NET_BIND_SERVICE
# (it nests a second user namespace), so the outer bwrap owns the network namespace, runs the loopback listeners
# with just that capability (plus CAP_SETFCAP, which the inner bwrap needs to start), and the inner bwrap holds the
# command with no capabilities and the same pid and network namespaces.
"$BWRAP" --ro-bind / / --dev /dev --proc /proc --unshare-user --uid 0 --gid 0 \
  --cap-add CAP_NET_BIND_SERVICE --cap-add CAP_SETFCAP --unshare-net --unshare-pid --die-with-parent \
  --bind "$DIR" "$DIR" -- \
  "$NODE" "$HERE/netproxy.mjs" run --sock "$DIR/net/proxy.sock" --drain "$DRAIN" "${FWD[@]}" -- \
  "$BWRAP" "${BW[@]}" --unshare-user --uid "$UID_" --gid "$GID_" -- "$@"
exit $?
