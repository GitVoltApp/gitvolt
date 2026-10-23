#!/usr/bin/env bash
# Usage: scripts/licenses.sh [out-dir]   (default: target/licenses)
#
# The license files the packages ship that don't come from the UI build (the UI build writes
# THIRD-PARTY-NOTICES-ui.txt itself, see ui/build/licenses.ts):
#   THIRD-PARTY-NOTICES-rust.txt  every crate gitvolt-app links, grouped by license (cargo-about,
#                                 then scripts/notices.py); fails on a license about.toml doesn't
#                                 accept, so a copyleft or unknown license stops the build
#   CEF-LICENSE.txt               CEF's own license, from the CEF distribution the build uses
#   CHROMIUM-CREDITS.html.gz         Chromium's credits for everything it bundles, from the same place
#   LICENSE                       GitVolt's license
#   DICTIONARY-en-US-LICENSE.txt  the spell-check dictionary's source and license (SCOWL), a copy
#                                 of crates/gitvolt-app/dictionaries/en-US-LICENSE.txt
# `just package` runs this before building, scripts/package-windows.ps1 in Git for Windows'
# bash (the Windows graph and CEF build, with Python as $PYTHON), and scripts/package-macos.sh on
# macOS (this Mac's architecture); docs/licensing.md describes the whole flow.
#
# --if-changed (`just package`): does nothing when the files in out-dir were made from the same
# inputs (Cargo.lock, every Cargo.toml, about.toml, the CEF distribution, these scripts and the
# license files copied) and haven't changed since; the key is in <out-dir>.key. A change to any of
# them reruns cargo-about, so a dependency under a license about.toml refuses still fails the
# build. GITVOLT_REBUILD=1, and every run without the flag (`just licenses`), regenerates.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
if_changed=
[ "${1:-}" = --if-changed ] && { if_changed=1; shift; }
[ "${GITVOLT_REBUILD:-}" = 1 ] && if_changed=
out=${1:-$root/target/licenses}
outputs=(THIRD-PARTY-NOTICES-rust.txt CEF-LICENSE.txt CHROMIUM-CREDITS.html.gz LICENSE DICTIONARY-en-US-LICENSE.txt)
fail() { echo "licenses: $*" >&2; exit 1; }
command -v cargo-about >/dev/null ||
  fail "cargo-about isn't installed: cargo install cargo-about --version 0.9.2 --locked --features cli"
mkdir -p "$out"
python=${PYTHON:-python3}
# The graph and the CEF build of the platform the package is for: about.toml's targets (Linux),
# Windows' in Git for Windows' bash, or this Mac's. The Tauri CLI's CEF folder is
# ~/Library/Caches/tauri-cef on macOS.
cef_cache=${XDG_CACHE_HOME:-$HOME/.cache}/tauri-cef
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) target=(--target x86_64-pc-windows-msvc) cef_platform=windows_x86_64 ;;
  Darwin)
    case "$(uname -m)" in
      arm64) target=(--target aarch64-apple-darwin) cef_platform=macos_aarch64 ;;
      *) target=(--target x86_64-apple-darwin) cef_platform=macos_x86_64 ;;
    esac
    cef_cache=$HOME/Library/Caches/tauri-cef ;;
  *) target=() cef_platform=linux_x86_64 ;;
esac

# CEF and Chromium: the distribution `cargo tauri` builds against. It sets CEF_PATH to
# $CEF_PATH or ~/.cache/tauri-cef, and cef-dll-sys unpacks CEF <version> into <that>/<version>/.
ver=$("$python" "$here/notices.py" cef-version "$root/Cargo.lock")
base=${CEF_PATH:-$cef_cache}
cef_dir=$base/$ver/cef_$cef_platform
[ -f "$base/CREDITS.html" ] && [ ! -d "$base/$ver" ] && cef_dir=$base # CEF_PATH = an unpacked distribution
if [ ! -f "$cef_dir/CREDITS.html" ]; then
  # A first build on this machine: download CEF the way the app build does (cef-dll-sys's build
  # script), so the package build that follows finds it in place.
  echo "licenses: CEF $ver isn't in $base yet; downloading it through cef-dll-sys's build" >&2
  (cd "$root" && CEF_PATH=$base cargo build --release --locked -p cef-dll-sys)
fi

# --if-changed: what the files are made from, and what they were when made from it.
key_file=$out.key
if [ -n "$if_changed" ]; then
  inputs_key() {
    {
      echo "target=${target[*]-} cef=$cef_dir $(cargo about --version)"
      (cd "$root" && find . -name Cargo.toml -not -path './target/*' -not -path '*/node_modules/*' -not -path './.claude/*' |
        LC_ALL=C sort | xargs sha256sum Cargo.lock about.toml scripts/licenses.sh scripts/notices.py LICENSE \
          crates/gitvolt-app/dictionaries/en-US-LICENSE.txt)
      sha256sum "$cef_dir/CREDITS.html"
      ls -l "$cef_dir"/../*.tar.bz2 2>/dev/null || true # the archive notices.py reads LICENSE.txt from
    } | sha256sum | cut -d' ' -f1
  }
  outputs_key() { (cd "$out" && sha256sum "${outputs[@]}" 2>&1) | sha256sum | cut -d' ' -f1; }
  key="$(inputs_key) $(outputs_key)"
  if [ -f "$key_file" ] && [ "$(cat "$key_file")" = "$key" ]; then
    echo "licenses: $out is up to date (same Cargo.lock, manifests, about.toml and CEF; GITVOLT_REBUILD=1 regenerates)"
    exit 0
  fi
fi
rm -f "$key_file"

# Rust. --locked: the notices describe exactly what Cargo.lock builds.
cargo about generate --format json --locked --fail -c "$root/about.toml" ${target[@]+"${target[@]}"} \
  -m "$root/crates/gitvolt-app/Cargo.toml" -o "$out/about.json"
"$python" "$here/notices.py" rust "$out/about.json" "$out/THIRD-PARTY-NOTICES-rust.txt"
rm "$out/about.json"

"$python" "$here/notices.py" cef "$cef_dir" "$out"

cp "$root/LICENSE" "$out/LICENSE"
cp "$root/crates/gitvolt-app/dictionaries/en-US-LICENSE.txt" "$out/DICTIONARY-en-US-LICENSE.txt"
for f in "${outputs[@]}"; do
  [ -s "$out/$f" ] || fail "$out/$f is missing or empty"
done
[ -z "$if_changed" ] || echo "$(inputs_key) $(outputs_key)" > "$key_file"
echo "licenses: wrote $(cd "$out" && ls | tr '\n' ' ')to $out"
echo "licenses: Rust crates by license:"
sed -n '/^Summary$/,/^$/p' "$out/THIRD-PARTY-NOTICES-rust.txt" | tail -n +3
