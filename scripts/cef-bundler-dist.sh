#!/usr/bin/env bash
# Usage: scripts/cef-bundler-dist.sh <dir> — prints the CEF_PATH for `cargo tauri build --bundles deb`.
#
# tauri-bundler 3.0.0-alpha.4 (tauri-cli 3.0.0-alpha.5) copies libEGL.so and libGLESv2.so from the
# CEF distribution into the .deb and fails ("No such file or directory") when they're missing.
# CEF 154 doesn't ship them any more: ANGLE is linked into libcef.so, and the two libraries only
# served `--use-dynamic-angle` (Chromium's ui/ozone/common/egl_util.cc), which GitVolt never passes.
# For such a distribution this makes <dir>/<version>/cef_linux_x86_64: links to the real
# distribution's files, plus two empty shared objects under those names for the bundler to copy.
# The package then carries them; with them or without, --use-dynamic-angle finds no ANGLE, so the
# app behaves the same. A distribution that has both libraries is used as it is.
# docs/upstream/tauri-bundler-cef-angle-libs.md has the upstream report draft.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
fail() { echo "cef-bundler-dist: $*" >&2; exit 1; }
out=$(realpath -m "${1:?usage: cef-bundler-dist.sh <dir>}")
ver=$(python3 "$here/notices.py" cef-version "$root/Cargo.lock")
# Where `cargo tauri` looks (scripts/licenses.sh, which runs first, has put CEF there).
base=${CEF_PATH:-${XDG_CACHE_HOME:-$HOME/.cache}/tauri-cef}
dist=$base/$ver/cef_linux_x86_64
[ -f "$dist/libcef.so" ] || fail "no CEF $ver in $dist (scripts/licenses.sh downloads it)"
if [ -e "$dist/libEGL.so" ] && [ -e "$dist/libGLESv2.so" ]; then
  echo "$base"
  exit 0
fi
stage=$out/$ver/cef_linux_x86_64
# Kept when it's already this distribution's: cef-dll-sys rebuilds (and with it everything that
# links CEF) when a file under CEF_PATH is newer than its last build.
staged() {
  [ -d "$stage" ] && [ "$(ls "$out")" = "$ver" ] || return 1
  [ "$(LC_ALL=C ls "$stage")" = "$( (ls "$dist"; echo libEGL.so; echo libGLESv2.so) | LC_ALL=C sort -u)" ] || return 1
  local f
  for f in "$dist"/*; do [ "$(readlink "$stage/${f##*/}")" = "$f" ] || return 1; done
  for f in libEGL.so libGLESv2.so; do [ -e "$dist/$f" ] || [ -f "$stage/$f" ] || return 1; done
}
if staged; then
  echo "$out"
  exit 0
fi
rm -rf "$out"
mkdir -p "$stage"
ln -s "$dist"/* "$stage"/
for lib in libEGL.so libGLESv2.so; do
  [ -e "$stage/$lib" ] || cc -shared -nostdlib -Wl,-soname,"$lib" -o "$stage/$lib" -x c /dev/null ||
    fail "can't build the empty $lib (needs a C compiler)"
done
echo "$out"
