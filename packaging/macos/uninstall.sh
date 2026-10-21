#!/bin/sh
# Removes Claude Switch installed from the macOS .pkg.
#
#   cswitch uninstall                                   # first, as yourself: PATH, shortcuts, ~/.cswitch/bin
#   sudo /usr/local/claude-switch/uninstall.sh          # then this: the package itself
set -eu
prefix=/usr/local/claude-switch

if [ "$(id -u)" -ne 0 ]; then
  echo "Run with sudo: sudo $0" >&2
  exit 1
fi

# Only remove links that point into our folder.
for name in /usr/local/bin/cswitch \
  /usr/local/share/zsh/site-functions/_cswitch \
  /usr/local/share/fish/vendor_completions.d/cswitch.fish \
  /usr/local/share/bash-completion/completions/cswitch; do
  if [ -L "$name" ]; then
    case "$(readlink "$name")" in
      "$prefix"/*) rm -f "$name" ;;
    esac
  fi
done
rm -rf "$prefix"
pkgutil --forget io.github.samrand96.claude-switch >/dev/null 2>&1 || true
echo "Claude Switch removed. Profile data stays in ~/.cswitch (delete it to remove your profiles)."
