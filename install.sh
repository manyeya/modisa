#!/bin/sh
# Install shepherd: curl -fsSL https://manyeya.github.io/shepherd/install.sh | sh
#   SHEPHERD_INSTALL_DIR  where the binary goes (default ~/.local/bin)
#   SHEPHERD_CHANNEL      stable (default) or staging for prerelease builds
#   SHEPHERD_MANIFEST_URL a release manifest to install from instead
#   NO_COLOR              plain output. It's plain anyway when not writing to a terminal; SHEPHERD_FANCY=1 forces the show.
# Remove it again: curl -fsSL https://manyeya.github.io/shepherd/install.sh | sh -s -- --uninstall [--purge]
set -eu

REPO="manyeya/shepherd"
DIR="${SHEPHERD_INSTALL_DIR:-$HOME/.local/bin}"
CHANNEL="${SHEPHERD_CHANNEL:-stable}"

# ---------- presentation: a show on a colour terminal, the same plain lines everywhere else ----------
FANCY=0
if [ "${SHEPHERD_FANCY:-}" = 1 ]; then FANCY=1
elif [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then FANCY=1; fi

E="$(printf '\033')"
R= B= DIM= GREEN= RED= VIOLET= CYAN= TRUE=0 NAP=0
if [ "$FANCY" = 1 ]; then
  case "${COLORTERM:-}" in *truecolor* | *24bit*) TRUE=1 ;; esac
  R="$E[0m" B="$E[1m" DIM="$E[2m"
  if [ "$TRUE" = 1 ]; then CYAN="$E[38;2;94;231;239m" VIOLET="$E[38;2;179;154;255m" GREEN="$E[38;2;165;239;181m" RED="$E[38;2;255;127;150m"
  else CYAN="$E[38;5;123m" VIOLET="$E[38;5;183m" GREEN="$E[38;5;157m" RED="$E[38;5;211m"; fi
  if sleep 0.01 2>/dev/null; then NAP=1; fi # fractional sleep, for the animations
fi

say() { printf '  %s\n' "$*"; }
die() {
  if [ "$FANCY" = 1 ]; then printf '\r%s[K' "$E"; fi
  printf '  %serror:%s %s\n' "$RED" "$R" "$*" >&2
  exit 1
}
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required"; }
nap() { if [ "$NAP" = 1 ]; then sleep "$1"; fi; }

# the brand gradient, from ion's cyan to its violet, at stop $1 of 0..7
shade() {
  if [ "$TRUE" = 1 ]; then printf '%s[38;2;%s;%s;%sm' "$E" $((94 + 85 * $1 / 7)) $((231 - 77 * $1 / 7)) $((239 + 16 * $1 / 7))
  else
    case $1 in 0) n=123 ;; 1 | 2) n=117 ;; 3 | 4) n=153 ;; 5 | 6) n=147 ;; *) n=183 ;; esac
    printf '%s[38;5;%sm' "$E" "$n"
  fi
}

# the wordmark, revealed letter by letter in the gradient, then the tagline typed out
banner() {
  echo
  if [ "$FANCY" != 1 ]; then say "shepherd · a terminal for your agents"; return; fi
  printf '%s[?25l' "$E"
  set -f
  for row in \
    '▄▀▀▀▀|█   █|█▀▀▀▀|█▀▀▀▄|█   █|█▀▀▀▀|█▀▀▀▄|█▀▀▀▄' \
    ' ▀▀▀▄|█▀▀▀█|█▀▀▀ |█▄▄▄▀|█▀▀▀█|█▀▀▀ |█▄▄▄▀|█   █' \
    '▄▄▄▄▀|█   █|█▄▄▄▄|█    |█   █|█▄▄▄▄|█  ▀▄|█▄▄▄▀'; do
    printf '  '
    i=0
    old_ifs=$IFS
    IFS='|'
    for letter in $row; do
      printf '%s%s%s ' "$(shade "$i")" "$letter" "$R"
      i=$((i + 1))
      nap 0.015
    done
    IFS=$old_ifs
    printf '\n'
  done
  set +f
  printf '\n  '
  tag="a terminal for your agents"
  while [ -n "$tag" ]; do
    rest=${tag#?}
    printf '%s%s%s' "$DIM" "${tag%"$rest"}" "$R"
    tag=$rest
    nap 0.012
  done
  printf '\n\n'
}

# step "label" command…: plain, it just runs the command. On a colour terminal it spins until the command is done, then
# shows ✓ or ✗. While WATCH names a file, the spinner shows how much of it has been written.
FRAMES='⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏'
WATCH=
size() { awk -v b="$(wc -c <"$1" | tr -d ' ')" 'BEGIN { printf "%.1f MB", b / 1048576 }'; }
step() {
  label=$1
  shift
  if [ "$FANCY" != 1 ]; then "$@"; return; fi
  "$@" >"$TMP/step.log" 2>&1 &
  pid=$!
  while kill -0 "$pid" 2>/dev/null; do
    for frame in $FRAMES; do
      extra=
      if [ -n "$WATCH" ] && [ -f "$WATCH" ]; then extra="  $DIM$(size "$WATCH")$R"; fi
      printf '\r%s[K  %s%s%s %s%s' "$E" "$CYAN" "$frame" "$R" "$label" "$extra"
      if [ "$NAP" = 1 ]; then sleep 0.08; else sleep 1; fi
      kill -0 "$pid" 2>/dev/null || break
    done
  done
  if wait "$pid"; then
    printf '\r%s[K  %s✓%s %s\n' "$E" "$GREEN" "$R" "$label"
  else
    printf '\r%s[K  %s✗%s %s\n' "$E" "$RED" "$R" "$label"
    return 1
  fi
}

if [ "${1:-}" = "--uninstall" ]; then
  shift
  BIN="$DIR/shepherd"
  [ -x "$BIN" ] || BIN="$(command -v shepherd || true)"
  [ -n "$BIN" ] || die "shepherd isn't installed: nothing in $DIR or on your PATH"
  # the binary does the work: integrations, sessions and state first, then itself. Under curl | sh
  # stdin is this script, so its "Continue?" question reads the terminal instead, when there is one.
  if (exec </dev/tty) 2>/dev/null; then exec "$BIN" uninstall "$@" </dev/tty; fi
  exec "$BIN" uninstall "$@"
fi

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

TMP="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP"
  if [ "$FANCY" = 1 ]; then printf '%s[?25h' "$E"; fi # the cursor comes back, however this ends
}
trap 'status=$?; cleanup; exit "$status"' EXIT # keep the exit status: a failed install must not exit 0
trap 'exit 130' INT TERM

banner
if [ "$FANCY" = 1 ]; then
  printf '  %sinstalling for%s %s%s%s %s(%s)%s\n\n' "$DIM" "$R" "$B" "$PLATFORM" "$R" "$DIM" "$CHANNEL" "$R"
else
  say "installing for $PLATFORM ($CHANNEL)"
fi

step "fetching the release manifest" curl -fsSL --retry 3 --connect-timeout 10 -o "$TMP/manifest.json" "$MANIFEST_URL" || die "couldn't fetch $MANIFEST_URL"
MANIFEST="$(cat "$TMP/manifest.json")"

# the manifest is written one field per line by the release workflow; read this platform's entry
field() { printf '%s\n' "$MANIFEST" | awk -v p="\"$PLATFORM\"" -v k="\"$1\"" '
  index($0, p) { inside = 1 }
  inside && index($0, k) { sub(/^[^:]*:[[:space:]]*"/, ""); sub(/".*$/, ""); print; exit }'; }
VERSION="$(printf '%s\n' "$MANIFEST" | awk -F'"' '/"version"/ { print $4; exit }')"
URL="$(field url)"
SHA="$(field sha256)"
[ -n "$URL" ] && [ -n "$SHA" ] || die "shepherd $VERSION has no build for $PLATFORM"

[ "$FANCY" = 1 ] || say "downloading shepherd $VERSION"
WATCH="$TMP/shepherd"
step "downloading shepherd $VERSION" curl -fsSL --retry 3 -o "$TMP/shepherd" "$URL" || die "download failed: $URL"
WATCH=

verify() { [ "$(sum "$TMP/shepherd")" = "$SHA" ]; }
step "verifying its SHA-256 checksum" verify || die "the download doesn't match its published checksum; nothing was installed"

place() { mkdir -p "$DIR" && chmod 755 "$TMP/shepherd" && mv -f "$TMP/shepherd" "$DIR/shepherd"; }
step "installing to $DIR" place || die "couldn't install to $DIR"

if [ "$FANCY" != 1 ]; then
  say "installed $DIR/shepherd"
  case ":$PATH:" in
    *":$DIR:"*) ;;
    *) say "add it to your PATH:  export PATH=\"$DIR:\$PATH\"" ;;
  esac
  say "start with:  shepherd    ·   update later with:  shepherd update"
  echo
  exit 0
fi

rail="${VIOLET}│${R}" # braces: a shell in a C locale would read the │ bytes as part of the name
printf '\n  %s╭─%s %sready%s\n' "$VIOLET" "$R" "$B" "$R"
printf '  %s %s✓%s shepherd %s%s%s installed  %s%s%s\n' "$rail" "$GREEN" "$R" "$B" "$VERSION" "$R" "$DIM" "$DIR/shepherd" "$R"
case ":$PATH:" in
  *":$DIR:"*) ;;
  *) printf '  %s %s!%s add it to your PATH:  %sexport PATH="%s:$PATH"%s\n' "$rail" "$VIOLET" "$R" "$B" "$DIR" "$R" ;;
esac
printf '  %s\n' "$rail"
printf '  %s   %sstart%s     shepherd\n' "$rail" "$CYAN" "$R"
printf '  %s   %supdate%s    shepherd update\n' "$rail" "$CYAN" "$R"
printf '  %s   %sdocs%s      https://manyeya.github.io/shepherd\n' "$rail" "$CYAN" "$R"
printf '  %s╰────────────────────────────────────────%s\n\n' "$VIOLET" "$R"
