#!/bin/sh
# Download the pinned Torpor engine for this platform into .engine/torpor; scripts/verify-engine.sh checks its sha256
# against engine.lock, and its `torpor version` must match the lock's version line.
set -eu
cd "$(dirname "$0")/.."
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) p=linux-x64 ;;
  Linux-aarch64 | Linux-arm64) p=linux-arm64 ;;
  Darwin-x86_64) p=darwin-x64 ;;
  Darwin-arm64) p=darwin-arm64 ;;
  *) echo "fetch-engine: no pinned binary for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
commit=$(awk '$1 == "commit" { print $2; exit }' engine.lock)
version=$(awk '$1 == "version" { print $2; exit }' engine.lock)
if [ -z "$commit" ] || [ -z "$version" ]; then echo "fetch-engine: engine.lock lacks the commit or version line" >&2; exit 1; fi
mkdir -p .engine
curl -fsSL -o .engine/torpor.part "https://raw.githubusercontent.com/Kylne700/torpor/$commit/dist/$p/torpor"
sh scripts/verify-engine.sh .engine/torpor.part "$p"
chmod 755 .engine/torpor.part
mv .engine/torpor.part .engine/torpor
got=$(.engine/torpor version)
if [ "$got" != "torpor $version" ]; then echo "fetch-engine: engine prints '$got', engine.lock pins $version" >&2; exit 1; fi
echo "$got"
