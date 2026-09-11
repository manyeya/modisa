#!/bin/sh
# Install shepherd: curl -fsSL https://manyeya.github.io/shepherd/install.sh | sh
#   SHEPHERD_INSTALL_DIR  where the binary goes (default ~/.local/bin)
#   SHEPHERD_CHANNEL      stable (default) or staging for prerelease builds
#   SHEPHERD_MANIFEST_URL a release manifest to install from instead
set -eu

REPO="manyeya/shepherd"
DIR="${SHEPHERD_INSTALL_DIR:-$HOME/.local/bin}"
CHANNEL="${SHEPHERD_CHANNEL:-stable}"

say() { printf '  %s\n' "$*"; }
die() { printf '  error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required"; }

case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  *) die "shepherd releases are for macOS and Linux; on $(uname -s), run it from source: https://github.com/$REPO" ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) arch=arm64 ;;
  x86_64 | amd64) arch=x64 ;;
  *) die "no shepherd release for $(uname -m); run it from source: https://github.com/$REPO" ;;
esac
[ "$os-$arch" = darwin-x64 ] && die "no Intel Mac build yet; run shepherd from source: https://github.com/$REPO"
PLATFORM="$os-$arch"

need curl
if command -v sha256sum >/dev/null 2>&1; then sum() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then sum() { shasum -a 256 "$1" | awk '{print $1}'; }
else die "sha256sum or shasum is required to verify the download"; fi

if [ -n "${SHEPHERD_MANIFEST_URL:-}" ]; then MANIFEST_URL="$SHEPHERD_MANIFEST_URL"
elif [ "$CHANNEL" = staging ]; then MANIFEST_URL="https://github.com/$REPO/releases/download/staging/manifest.json"
else MANIFEST_URL="https://github.com/$REPO/releases/latest/download/manifest.json"; fi

echo
say "shepherd · a terminal for your agents"
say "installing for $PLATFORM ($CHANNEL)"
MANIFEST="$(curl -fsSL --retry 3 --connect-timeout 10 "$MANIFEST_URL")" || die "couldn't fetch $MANIFEST_URL"

# the manifest is written one field per line by the release workflow; read this platform's entry
field() { printf '%s\n' "$MANIFEST" | awk -v p="\"$PLATFORM\"" -v k="\"$1\"" '
  index($0, p) { inside = 1 }
  inside && index($0, k) { sub(/^[^:]*:[[:space:]]*"/, ""); sub(/".*$/, ""); print; exit }'; }
VERSION="$(printf '%s\n' "$MANIFEST" | awk -F'"' '/"version"/ { print $4; exit }')"
URL="$(field url)"
SHA="$(field sha256)"
[ -n "$URL" ] && [ -n "$SHA" ] || die "shepherd $VERSION has no build for $PLATFORM"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM
say "downloading shepherd $VERSION"
curl -fsSL --retry 3 -o "$TMP/shepherd" "$URL" || die "download failed: $URL"
[ "$(sum "$TMP/shepherd")" = "$SHA" ] || die "the download doesn't match its published checksum; nothing was installed"

mkdir -p "$DIR"
chmod 755 "$TMP/shepherd"
mv -f "$TMP/shepherd" "$DIR/shepherd"
say "installed $DIR/shepherd"

case ":$PATH:" in
  *":$DIR:"*) ;;
  *) say "add it to your PATH:  export PATH=\"$DIR:\$PATH\"" ;;
esac
say "start with:  shepherd    ·   update later with:  shepherd update"
echo
