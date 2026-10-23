#!/usr/bin/env bash
# Self-test for scripts/package-compression.sh: a release build (GITVOLT_RELEASE_VERSION, or
# GITVOLT_PACKAGE_COMPRESSION=release) keeps what the release has always shipped (Tauri's gzip 6
# .deb, zstd -19 Arch package and RPM), every other build is fast, and a bad mode fails.
set -euo pipefail
helper="$(cd "$(dirname "$0")" && pwd)/package-compression.sh"
fail() { echo "test-package-compression: FAIL: $*" >&2; exit 1; }
run() { env -u GITVOLT_RELEASE_VERSION -u GITVOLT_PACKAGE_COMPRESSION "$@"; }
eq() { [ "$2" = "$3" ] || fail "$1: expected '$3', got '$2'"; }
eq local-deb "$(run "$helper" deb)" '{"type":"zstd","level":3}'
eq local-zstd "$(run "$helper" zstd)" 3
eq release-deb "$(run GITVOLT_RELEASE_VERSION=1.2.3 "$helper" deb)" '{"type":"gzip","level":6}'
eq release-zstd "$(run GITVOLT_RELEASE_VERSION=1.2.3 "$helper" zstd)" 19
eq forced-release "$(run GITVOLT_PACKAGE_COMPRESSION=release "$helper" zstd)" 19
eq forced-fast "$(run GITVOLT_RELEASE_VERSION=1.2.3 GITVOLT_PACKAGE_COMPRESSION=fast "$helper" zstd)" 3
if run GITVOLT_PACKAGE_COMPRESSION=best "$helper" zstd 2>/dev/null; then fail "accepted an unknown mode"; fi
if run "$helper" xz 2>/dev/null; then fail "accepted an unknown format"; fi
echo "test-package-compression: OK"
