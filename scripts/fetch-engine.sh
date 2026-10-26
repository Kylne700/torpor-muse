#!/bin/sh
# Download the pinned Torpor engine for this platform into .engine/torpor; fail unless its sha256 matches engine.lock.
set -eu
cd "$(dirname "$0")/.."
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) p=linux-x64 ;;
  Linux-aarch64 | Linux-arm64) p=linux-arm64 ;;
  Darwin-x86_64) p=darwin-x64 ;;
  Darwin-arm64) p=darwin-arm64 ;;
  *) echo "fetch-engine: no pinned binary for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
val() { sed -n "s/^$1 = \"\(.*\)\"\$/\1/p" engine.lock; }
commit=$(val commit)
want=$(val "$p")
mkdir -p .engine
curl -fsSL -o .engine/torpor.part "https://raw.githubusercontent.com/Kylne700/torpor/$commit/dist/$p/torpor"
got=$( (sha256sum .engine/torpor.part 2>/dev/null || shasum -a 256 .engine/torpor.part) | cut -c1-64)
if [ "$got" != "$want" ]; then echo "fetch-engine: sha256 mismatch for $p: $got" >&2; exit 1; fi
chmod 755 .engine/torpor.part
mv .engine/torpor.part .engine/torpor
.engine/torpor version
