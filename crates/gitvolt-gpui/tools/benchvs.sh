#!/usr/bin/env bash
# GPUI against a release of the CEF build, by turns, each in a nested compositor of its own, from the repository's root: the CEF
# package's unpacked binary, GPUI under Xwayland (as CEF runs) and GPUI on Wayland. A build's first round warms the disk and its
# caches, and doesn't count.
# usage: benchvs.sh <rounds> <cef binary> <out.jsonl>   (BENCH_ONLY: one series; BENCH_FILE_X, BENCH_FILE_Y: the working copy's file row)
set -u
grep -q '/run-[^/]*\.scope$' /proc/self/cgroup || exec "$(dirname "$0")/capped.sh" "$0" "$@"
rounds=$1 cef=$2 out=$3
tools=$(cd "$(dirname "$0")" && pwd)
source "$tools/flows/lib.sh"
repo=$WORK/flows/bench
[ -d "$repo/.git" ] || repo=$(long_repo bench 4000)
gpui=$(pwd)/target/bench/release/gitvolt-gpui
settings='{"version":2,"gravatar":false,"forgeAvatars":false,"updateCheck":false,"fetchIntervalSecs":0}'
series() { # label binary native
  local root=$WORK/tmp/bench-$1
  mkdir -p "$root/app/config/gitvolt-gpui" "$root/app/config/gitvolt"
  [ -f "$root/app/config/gitvolt/settings.json" ] || { echo "$settings" >"$root/app/config/gitvolt/settings.json"; echo "$settings" >"$root/app/config/gitvolt-gpui/settings.json"; }
  # Each run from the same place: a saved position moves a framed window down by its frame, and the clicks would miss.
  python3 -c 'import json,sys
for f in sys.argv[1:]:
    d = json.load(open(f)); d.pop("window", None); json.dump(d, open(f, "w"))' "$root/app/config/gitvolt/settings.json" "$root/app/config/gitvolt-gpui/settings.json"
  # A machine this full measures the machine.
  for _ in $(seq 1 60); do [ "$(free -m | awk 'NR==2{print $7}')" -ge 2560 ] && break; sleep 30; done
  REALINPUT_ROOT=$root GITVOLT_GPUI_BIN=$2 GITVOLT_RUN_KB=unlimited NATIVE=$3 NESTED_W=1920 NESTED_H=1200 \
    timeout -k 5 150 "$tools/realinput.sh" - python3 "$tools/benchvs.py" "$1" "$repo" "${BENCH_FILE_X:-1400}" "${BENCH_FILE_Y:-400}" "$out.$4" "$SHOTS/bench"
}
: >"$out.warm"; : >"$out"
for r in $(seq 0 "$rounds"); do
  [ "$r" = 0 ] && dest=warm || dest=
  for cfg in "cef $cef" "gpui-x11 $gpui" "gpui-wayland $gpui 1"; do
    set -- $cfg
    [ -n "${BENCH_ONLY:-}" ] && [ "$1" != "$BENCH_ONLY" ] && continue
    series "$1" "$2" "${3:-}" "${dest:-run}" 2>&1 | tail -1 | cut -c1-400
  done
done
cat "$out.run" >>"$out" 2>/dev/null
echo "rounds done: $(wc -l <"$out")"
