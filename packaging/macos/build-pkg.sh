#!/bin/sh
# Builds the macOS installer package (.pkg) for one architecture.
#
#   packaging/macos/build-pkg.sh <cswitch binary> <version> <arm64|x86_64> <completions dir> <output dir>
#
# <completions dir> holds cswitch.bash, _cswitch and cswitch.fish (`cswitch completions …`).
#
# Layout: everything goes into /usr/local/claude-switch, a folder only this package owns; the
# postinstall script then links /usr/local/bin/cswitch (on every Mac's default PATH) and the
# shell completions. The payload deliberately contains no shared folders such as
# /usr/local/bin — Installer would re-own them, which breaks Homebrew on Intel Macs.
set -eu

if [ "$#" -ne 5 ]; then
  sed -n '2,6p' "$0" >&2
  exit 2
fi
bin="$1"
version="$2"
arch="$3"
completions="$4"
out="$5"

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
id="io.github.samrand96.claude-switch"

case "$arch" in
  arm64) min_os="11.0" ;;
  x86_64) min_os="10.13" ;;
  *) echo "unknown architecture: $arch (use arm64 or x86_64)" >&2; exit 2 ;;
esac
# Installer compares dotted numbers: "1.2.3-beta.1" -> "1.2.3".
pkg_version="${version%%-*}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
root="$work/root"
mkdir -p "$root/bin" "$root/share/zsh/site-functions" "$root/share/fish/vendor_completions.d" \
  "$root/share/bash-completion/completions" "$work/scripts" "$work/resources" "$out"

install -m 0755 "$bin" "$root/bin/cswitch"
install -m 0644 "$completions/_cswitch" "$root/share/zsh/site-functions/_cswitch"
install -m 0644 "$completions/cswitch.fish" "$root/share/fish/vendor_completions.d/cswitch.fish"
install -m 0644 "$completions/cswitch.bash" "$root/share/bash-completion/completions/cswitch"
install -m 0755 "$here/uninstall.sh" "$root/uninstall.sh"
install -m 0644 "$repo/LICENSE" "$root/LICENSE"
install -m 0644 "$repo/README.md" "$root/README.md"
install -m 0755 "$here/postinstall" "$work/scripts/postinstall"

pkgbuild \
  --root "$root" \
  --identifier "$id" \
  --version "$pkg_version" \
  --install-location /usr/local/claude-switch \
  --scripts "$work/scripts" \
  --ownership recommended \
  "$work/claude-switch.pkg"

sed -e "s/@VERSION@/$version/g" -e "s/@PKG_VERSION@/$pkg_version/g" \
  -e "s/@ARCH@/$arch/g" -e "s/@MIN_OS@/$min_os/g" -e "s/@ID@/$id/g" \
  "$here/distribution.xml" >"$work/distribution.xml"
cp "$here/welcome.html" "$here/conclusion.html" "$work/resources/"
cp "$repo/LICENSE" "$work/resources/LICENSE.txt"

product="$out/claude-switch-$version-macos-$arch.pkg"
productbuild \
  --distribution "$work/distribution.xml" \
  --resources "$work/resources" \
  --package-path "$work" \
  "$product"
echo "built $product"
