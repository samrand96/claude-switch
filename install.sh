#!/bin/sh
# Installs Claude Switch (cswitch) on macOS / Linux and starts the setup wizard.
#   curl -fsSL https://raw.githubusercontent.com/samrand96/claude-switch/HEAD/install.sh | sh
#   CSWITCH_BIN=./cswitch sh install.sh        # use a local binary
#   CSWITCH_VERSION=v1.2.3 sh install.sh       # a specific release (or "nightly") instead of the latest
#   CSWITCH_NO_SETUP=1 sh install.sh           # install only, no wizard
set -eu

REPO="${CSWITCH_REPO:-samrand96/claude-switch}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

say() { printf '  > %s\n' "$*"; }
fail() {
  printf '  x %s\n' "$*" >&2
  exit 1
}

download() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$2" "$1"
  else
    fail "curl or wget is required"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

if [ -n "${CSWITCH_BIN:-}" ]; then
  exe="$CSWITCH_BIN"
else
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$arch" in
    x86_64 | amd64) arch="x86_64" ;;
    arm64 | aarch64) arch="aarch64" ;;
    *) fail "unsupported CPU: $arch (Claude Switch is built for x86_64 and arm64)" ;;
  esac
  case "$os" in
    Darwin)
      # A shell running under Rosetta reports x86_64 on Apple Silicon: install the native build.
      if [ "$arch" = "x86_64" ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = "1" ]; then
        arch="aarch64"
      fi
      target="$arch-apple-darwin"
      ;;
    Linux) target="$arch-unknown-linux-musl" ;; # static: works on every distribution
    *) fail "unsupported OS: $os (on Windows use install.ps1 or the setup .exe)" ;;
  esac
  asset="cswitch-$target.tar.gz"
  if [ -n "${CSWITCH_VERSION:-}" ]; then
    base="https://github.com/$REPO/releases/download/$CSWITCH_VERSION"
    say "Downloading $base/$asset"
    download "$base/$asset" "$work/$asset"
  else
    base="https://github.com/$REPO/releases/latest/download"
    say "Downloading $base/$asset"
    if ! download "$base/$asset" "$work/$asset" 2>/dev/null; then
      # "latest" skips pre-releases: with no stable release yet, take the nightly.
      base="https://github.com/$REPO/releases/download/nightly"
      say "No stable release yet, downloading the nightly build"
      download "$base/$asset" "$work/$asset"
    fi
  fi
  if download "$base/SHA256SUMS.txt" "$work/SHA256SUMS.txt" 2>/dev/null; then
    expected="$(grep " $asset\$" "$work/SHA256SUMS.txt" | cut -d ' ' -f 1)"
    actual="$(sha256 "$work/$asset")"
    if [ -n "$expected" ] && [ -n "$actual" ]; then
      [ "$expected" = "$actual" ] || fail "checksum mismatch for $asset — download again"
      say "Checksum verified"
    fi
  fi
  tar -xzf "$work/$asset" -C "$work"
  exe="$work/cswitch"
fi

chmod +x "$exe"
say "Installing to ${CSWITCH_HOME:-$HOME/.cswitch}/bin"
"$exe" install
installed="${CSWITCH_HOME:-$HOME/.cswitch}/bin/cswitch"

if [ -z "${CSWITCH_NO_SETUP:-}" ]; then
  if [ -t 0 ]; then
    "$installed" setup
  elif [ -t 1 ] && (: </dev/tty) 2>/dev/null; then
    # `curl … | sh`: stdin is this script, so the wizard talks to the terminal directly.
    "$installed" setup </dev/tty
  else
    say "Run 'cswitch setup' to create your profiles."
  fi
fi
echo "  Done. Open a new terminal and run: cswitch"
