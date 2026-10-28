rel=https://github.com/Kylne700/torpor-muse/releases/download/v0.1.0
eng=https://raw.githubusercontent.com/Kylne700/torpor/a2289f4c3cdc7779f18cda56f95c607bb01a88cc/dist
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) p=linux-x64 e=0048fc3b7ee63f3d77c9f3cb63941176b30ddd427f7f6dd8dae929241c7f2e3f ;;
  Linux-aarch64 | Linux-arm64) p=linux-arm64 e=0e6e61341e735319b8a5d857031324c192ea675b8d9b4fb29e96672dc6039c7d ;;
  Darwin-x86_64) p=darwin-x64 e=8b4bbf292026537b0b84db69f3c0c0463650ca76e3d21dd93863d51c40c184dc ;;
  Darwin-arm64) p=darwin-arm64 e=0ffd3a1c05f8ddefef4ffe8c04da6c4118625cb3a5465af94a359624650a948b ;;
  *) echo "no torpor-muse build for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
check() { if command -v sha256sum >/dev/null 2>&1; then sha256sum -c "$1"; else shasum -a 256 -c "$1"; fi; }
d=$(mktemp -d)
trap 'rm -rf "$d"' EXIT
cd "$d"
curl -fsSL -o torpor "$eng/$p/torpor"
echo "$e  torpor" > torpor.sha256
check torpor.sha256
curl -fsSL -o "torpor-muse-$p" "$rel/torpor-muse-$p"
curl -fsSL -o SHA256SUMS "$rel/SHA256SUMS"
grep " torpor-muse-$p\$" SHA256SUMS > torpor-muse.sha256
check torpor-muse.sha256
mkdir -p "$HOME/.local/bin"
install -m 755 torpor "$HOME/.local/bin/torpor"
install -m 755 "torpor-muse-$p" "$HOME/.local/bin/torpor-muse"
