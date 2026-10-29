#!/usr/bin/env bash
# Usage: scripts/macos-keychain.sh import|delete   (the macOS jobs of release.yml and mac-port.yml)
#
# import: GitVolt's self-signed code-signing certificate and its key, from MACOS_CERTIFICATE_P12
# (the .p12, base64) and MACOS_CERTIFICATE_PASSWORD, into a throwaway keychain in $RUNNER_TEMP:
# unlocked for the whole build, in the search list, its key usable by codesign without a prompt.
# Then a test signature, and GITVOLT_MACOS_SIGNING_IDENTITY (the certificate's SHA-1, which
# scripts/package-macos.sh signs with) into $GITHUB_ENV. Without the secrets (a fork's run) it
# does nothing and the app is ad-hoc signed, unless GITVOLT_MACOS_SIGNING_REQUIRED is `true`.
# delete: deletes that keychain (run it with `if: always()`).
#
# The certificate isn't trusted on the runner and needn't be: codesign signs with it by its hash,
# and a signature verifies, `--strict` too, without trusting its certificate (docs/releasing.md,
# "macOS signing"). Nothing secret is printed: the keychain's random password is masked, and the
# .p12 is decoded into a private folder that is deleted on exit.
set -euo pipefail
fail() { echo "macos-keychain: $*" >&2; exit 1; }
[ "$(uname -s)" = Darwin ] || fail "runs on macOS only"
keychain=${RUNNER_TEMP:-${TMPDIR:-/tmp}}/gitvolt-signing.keychain-db

case ${1:-} in
  import)
    if [ -z "${MACOS_CERTIFICATE_P12:-}" ] || [ -z "${MACOS_CERTIFICATE_PASSWORD:-}" ]; then
      [ "${GITVOLT_MACOS_SIGNING_REQUIRED:-}" != true ] || fail "no MACOS_CERTIFICATE_P12 or MACOS_CERTIFICATE_PASSWORD secret"
      echo "macos-keychain: no signing certificate; the app will be ad-hoc signed"
      exit 0
    fi
    pass=$(openssl rand -hex 24)
    [ -z "${GITHUB_ACTIONS:-}" ] || echo "::add-mask::$pass"
    dir=$(mktemp -d)
    trap 'rm -rf "$dir"' EXIT
    printf '%s' "$MACOS_CERTIFICATE_P12" | base64 --decode > "$dir/cert.p12" 2>/dev/null ||
      fail "MACOS_CERTIFICATE_P12 isn't base64"
    security delete-keychain "$keychain" 2>/dev/null || true
    security create-keychain -p "$pass" "$keychain"
    # No timeout: a keychain locks itself after 5 minutes by default, long before the bundler signs.
    security set-keychain-settings "$keychain"
    security unlock-keychain -p "$pass" "$keychain"
    security import "$dir/cert.p12" -k "$keychain" -f pkcs12 -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null ||
      fail "couldn't import MACOS_CERTIFICATE_P12 (wrong password, or not a legacy-encrypted .p12?)"
    security set-key-partition-list -S apple-tool:,apple: -s -k "$pass" "$keychain" >/dev/null
    # First in the user's search list, the others kept.
    list=("$keychain")
    while IFS= read -r k; do
      k=${k#*\"}
      list+=("${k%\"}")
    done < <(security list-keychains -d user)
    security list-keychains -d user -s "${list[@]}"

    # One identity, untrusted here (CSSMERR_TP_NOT_TRUSTED): listed, but not as valid.
    identities=$(security find-identity -p codesigning "$keychain")
    hash=$(sed -n 's/^ *[0-9][0-9]*) \([0-9A-F]\{40\}\) .*/\1/p' <<<"$identities" | sort -u)
    [ "$(grep -c . <<<"$hash")" = 1 ] || fail "expected one code-signing identity in the certificate: $identities"
    echo "macos-keychain: $(grep -m1 "$hash" <<<"$identities" | sed 's/^ *[0-9]*) //')"
    cp /usr/bin/true "$dir/probe"
    codesign --force --sign "$hash" "$dir/probe" || fail "codesign can't sign with $hash"
    codesign --verify --strict "$dir/probe" || fail "a signature by $hash doesn't verify"
    if [ -n "${GITHUB_ENV:-}" ]; then
      echo "GITVOLT_MACOS_SIGNING_IDENTITY=$hash" >> "$GITHUB_ENV"
    else
      echo "export GITVOLT_MACOS_SIGNING_IDENTITY=$hash"
    fi
    ;;
  delete)
    security delete-keychain "$keychain" 2>/dev/null || true
    rm -f "$keychain"
    ;;
  *) fail "usage: macos-keychain.sh import|delete" ;;
esac
