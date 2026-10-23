#!/usr/bin/env bash
# Usage: scripts/package-compression.sh deb|zstd
#
# How hard the Linux packages are compressed: `deb` prints the .deb's compression as Tauri's
# bundle > linux > deb > compression JSON, `zstd` the zstd level of the Arch package and the RPM.
#   release  what the release workflow ships: gzip 6 (Tauri's default, which any dpkg reads) and
#            zstd -19. About 75 s of zstd per package
#   fast     local builds: zstd 3 for both, a few seconds each, and about 20% larger packages
#            (151 MiB, not 124-157). A zstd .deb needs dpkg 1.21.18 or later (Debian 12,
#            Ubuntu 22.04) to install
# GITVOLT_PACKAGE_COMPRESSION=release|fast picks one; unset, a release build
# (GITVOLT_RELEASE_VERSION set) is `release` and every other build `fast`.
set -euo pipefail
fail() { echo "package-compression: $*" >&2; exit 1; }
mode=${GITVOLT_PACKAGE_COMPRESSION:-}
[ -n "$mode" ] || { [ -n "${GITVOLT_RELEASE_VERSION:-}" ] && mode=release || mode=fast; }
case "$mode:${1:-}" in
  release:deb) echo '{"type":"gzip","level":6}' ;;
  release:zstd) echo 19 ;;
  fast:deb) echo '{"type":"zstd","level":3}' ;;
  fast:zstd) echo 3 ;;
  release:* | fast:*) fail "usage: package-compression.sh deb|zstd" ;;
  *) fail "GITVOLT_PACKAGE_COMPRESSION is '$mode', not release or fast" ;;
esac
