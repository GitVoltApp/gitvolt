#!/usr/bin/env bash
# Static checks of the Windows installers' sources (they build only on Windows: package-windows.ps1).
# The MSI keeps GitBolt's UpgradeCode (the app's name before 0.7.0): never change it.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
fail() { echo "test-windows-installers: FAIL: $*" >&2; exit 1; }
wxs="$root/packaging/windows/gitvolt.wxs"
nsi="$root/packaging/windows/gitvolt.nsi"
grep -qF 'UpgradeCode="7003EA59-7995-4744-BA0C-D01FB85D7BBA"' "$wxs" || fail "the MSI's UpgradeCode changed: GitBolt's MSI wouldn't be upgraded"
grep -qF '<MajorUpgrade ' "$wxs" || fail "the MSI has no MajorUpgrade"
[ "$(grep -cF 'IntCmp $3 20 ' "$nsi")" = 1 ] || fail "the installer doesn't wait for a quitting app before asking"
echo "test-windows-installers: OK"
