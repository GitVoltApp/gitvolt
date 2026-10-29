#!/usr/bin/env bash
# Usage: scripts/package-macos.sh   (`just package-macos`, on a Mac)
#
# The macOS package: target/release/bundle/dmg/GitVolt_<version>_<arch>.dmg (arch: aarch64 on
# Apple Silicon, x64 on Intel), holding GitVolt.app and a link to /Applications.
#
# 1. The license notices (scripts/licenses.sh: the Rust crates of this Mac's target, CEF and
#    Chromium), failing on a license outside about.toml's allow-list.
# 2. `cargo tauri build --bundles app,dmg`, release: the UI build (with its own notices), the app,
#    then the bundle (tauri.macos.conf.json: the CEF framework, the helper apps, the folder
#    document type, the signature applied inside out) and the .dmg, from Tauri's own dmg
#    bundler. packaging/macos/dmg.conf.json adds the package's files to Contents/Resources: the
#    `install-kind` marker (`dmg`, for the updater) and the notices in `licenses/`.
# 3. Checks: the signature (`codesign --verify --deep --strict`, the hardened runtime, CEF's
#    entitlements, the designated requirement, the helpers'), the marker, the notices and the
#    folder document type in the bundle, then the .dmg mounted read-only: GitVolt.app, signed,
#    and the Applications link.
#
# The signature: with GITVOLT_MACOS_SIGNING_IDENTITY (a code-signing identity in the keychain
# search list, by SHA-1 or name: the release's self-signed certificate, which
# scripts/macos-keychain.sh imports), that identity; without it, ad-hoc. Either way each helper
# app gets its own identifier (scripts/macos-codesign/codesign). See docs/releasing.md, "macOS
# signing".
#
# The version follows scripts/package-version.sh: a stamped one for local builds, the plain one
# with GITVOLT_RELEASE_VERSION (the release workflow). CARGO_BUILD_JOBS defaults to 4. Needs
# Xcode's command line tools, CMake, Ninja, Node, cargo-about and the pinned Tauri CLI
# (docs/dev-setup.md).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
fail() { echo "package-macos: $*" >&2; exit 1; }
[ "$(uname -s)" = Darwin ] || fail "builds on macOS only"
case "$(uname -m)" in
  arm64) arch=aarch64 ;;
  x86_64) arch=x64 ;;
  *) fail "unknown architecture $(uname -m)" ;;
esac

cd "$root"
scripts/licenses.sh
app_dir=crates/gitvolt-app
v=$(scripts/package-version.sh "$app_dir/tauri.conf.json")
bundle=target/release/bundle
app=$bundle/macos/GitVolt.app
dmg=$bundle/dmg/GitVolt_${v}_${arch}.dmg
identity=${GITVOLT_MACOS_SIGNING_IDENTITY:--}
bundle_id=$(node -p "require(\"./$app_dir/tauri.conf.json\").identifier")
# APPLE_SIGNING_IDENTITY overrides tauri.macos.conf.json's "-"; the bundler signs inside out (the
# helper apps, the framework's libraries and the framework, the main binary, then the bundle) and,
# with an identity, the .dmg too.
tauri() {
  (cd "$app_dir" && env -u CARGO_INCREMENTAL CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" GITVOLT_BUILD_VERSION="$v" \
    APPLE_SIGNING_IDENTITY="$identity" PATH="$here/macos-codesign:$PATH" \
    cargo tauri "$@" --config packaging/macos/dmg.conf.json --config "{\"version\":\"$v\"}")
}
# Old bundles first: the checks below must see only what this build makes.
rm -rf "$bundle/macos" "$bundle/dmg"
# `hdiutil create` sometimes fails with "Resource busy" (a scan of the fresh image still holds
# it; common on CI runners): once the app is built, the .dmg alone is tried twice more.
if ! tauri build --bundles app,dmg; then
  [ -d "$app" ] || fail "the build failed"
  for try in 2 3; do
    echo "package-macos: the .dmg failed; trying again ($try of 3)" >&2
    sleep 15
    if tauri bundle --bundles dmg; then break; fi
    [ "$try" != 3 ] || fail "the .dmg failed three times"
  done
fi
[ -d "$app" ] || fail "no $app"
[ -f "$dmg" ] || fail "no $dmg: $(ls "$bundle/dmg" 2>/dev/null | tr '\n' ' ')"

designated() { codesign -d -r- "$1" 2>/dev/null | sed -n 's/^designated => //p'; }
# codesign's default for a certificate outside Apple's: the identifier and the hash of the chain's
# root, which for a self-signed one is the certificate itself.
names_cert() {
  local req
  req=$(designated "$1")
  [ "$req" = "identifier \"$2\" and certificate root = H\"$3\"" ] || [ "$req" = "identifier \"$2\" and certificate leaf = H\"$3\"" ] ||
    fail "$1: the designated requirement is '$req', not its identifier and the certificate's hash"
}
leaf_sha1() {
  local d
  d=$(mktemp -d)
  codesign -d --extract-certificates="$d/cert" "$1" 2>/dev/null && shasum -a 1 "$d/cert0" | cut -c1-40
  rm -rf "$d"
}

# The signature, valid as `--strict` sees it, with the hardened runtime and CEF's entitlements
# (without disable-library-validation the runtime would refuse CEF's framework under any identity
# but Apple's). With an identity, the designated requirement, which Keychain and Privacy &
# Security remember the app by, must be the identifier and the certificate's hash: the same in
# every release. Ad-hoc, it's the cdhash, new with each build. The helpers: each its own
# identifier, and the same certificate.
check_signature() {
  local app=$1 info ents id leaf='' h hid hsig ids=() want
  codesign --verify --deep --strict "$app" || fail "$app: the signature doesn't verify"
  info=$(codesign -dvv "$app" 2>&1)
  grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' <<<"$info" || fail "$app: no hardened runtime"
  # codesign's output is captured, not piped to `grep -q`: that stops reading at the match, and
  # codesign's next write then fails the pipeline (pipefail).
  ents=$(codesign -d --entitlements - --xml "$app" 2>/dev/null)
  [[ $ents == *'<key>com.apple.security.cs.disable-library-validation</key>'* ]] ||
    fail "$app: no disable-library-validation entitlement"
  id=$(plutil -extract CFBundleIdentifier raw "$app/Contents/Info.plist")
  [ "$id" = "$bundle_id" ] || fail "$app: CFBundleIdentifier $id, not $bundle_id"
  if [ "$identity" = - ]; then
    grep -qx 'Signature=adhoc' <<<"$info" || fail "$app: not ad-hoc signed"
  else
    leaf=$(leaf_sha1 "$app")
    [ -n "$leaf" ] || fail "$app: no certificate in its signature"
    if [[ $identity =~ ^[0-9A-Fa-f]{40}$ ]] && [ "$(tr '[:upper:]' '[:lower:]' <<<"$identity")" != "$leaf" ]; then
      fail "$app: signed with the certificate $leaf, not $identity"
    fi
    names_cert "$app" "$id" "$leaf"
  fi
  for h in "$app"/Contents/Frameworks/*.app; do
    hid=$(plutil -extract CFBundleIdentifier raw "$h/Contents/Info.plist")
    ids+=("$hid")
    hsig=$(codesign -dv "$h" 2>&1 | sed -n 's/^Identifier=//p')
    [ "$hsig" = "$hid" ] || fail "$h: signed as '$hsig', not $hid"
    [ -z "$leaf" ] || names_cert "$h" "$hid" "$leaf"
  done
  want=$(printf '%s\n' "$id.helper" "$id.helper.alerts" "$id.helper.gpu" "$id.helper.plugin" "$id.helper.renderer")
  [ "$(printf '%s\n' "${ids[@]}" | sort)" = "$want" ] ||
    fail "$app: the helpers' identifiers are $(printf '%s ' "${ids[@]}")"
}

check_app() {
  local app=$1 res=$1/Contents/Resources
  check_signature "$app"
  [ "$(cat "$res/install-kind")" = dmg ] || fail "$app: no install-kind 'dmg' in Contents/Resources"
  for f in LICENSE THIRD-PARTY-NOTICES-rust.txt THIRD-PARTY-NOTICES-ui.txt CEF-LICENSE.txt CHROMIUM-CREDITS.html.gz DICTIONARY-en-US-LICENSE.txt; do
    [ -s "$res/licenses/$f" ] || fail "$app: licenses/$f is missing or empty"
  done
  [ -s "$res/dictionaries/en-US-10-2.bdic" ] || fail "$app: no spell-check dictionary"
  plutil -extract CFBundleDocumentTypes.0.LSItemContentTypes.0 raw "$app/Contents/Info.plist" | grep -qx public.folder ||
    fail "$app: Info.plist doesn't declare the folder document type"
  [ "$(plutil -extract CFBundleShortVersionString raw "$app/Contents/Info.plist")" = "$v" ] || fail "$app: not version $v"
}
check_app "$app"
codesign -dvv "$app" 2>&1 | grep -E '^(Identifier|Authority|TeamIdentifier|Signature|CodeDirectory)[= ]'
echo "designated => $(designated "$app")"

mnt=$(mktemp -d "${TMPDIR:-/tmp}/gitvolt-dmg.XXXXXX")
trap 'hdiutil detach -quiet "$mnt" 2>/dev/null || hdiutil detach -quiet -force "$mnt" 2>/dev/null || true; rmdir "$mnt" 2>/dev/null || true' EXIT
hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$mnt" "$dmg" >/dev/null
check_app "$mnt/GitVolt.app"
[ "$(readlink "$mnt/Applications")" = /Applications ] || fail "$dmg: no link to /Applications"
echo "package-macos: $dmg ($(du -h "$dmg" | cut -f1); GitVolt.app $(du -sh "$app" | cut -f1)), version $v"
