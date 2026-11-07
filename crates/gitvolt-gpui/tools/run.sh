#!/usr/bin/env bash
# The build on a fixture, isolated: its config, data and cache under target/bench/work/xdg, no avatars
# asked for. A script (GITVOLT_SCRIPT, bench.rs) runs its flow; a screenshot of this window comes at
# `shot <out.png>` lines' moments by `tools/shot.sh`'s way, when one's asked for with SHOT_AT.
# Under a memory cap and a timeout, so a runaway fails on its own: GITVOLT_RUN_VM (kB of address space, 16 GB by default;
# the GPU driver's mappings count), GITVOLT_RUN_SECS. GITVOLT_XDG: another profile than the default, for a clean run.
# usage: run.sh <repo> [script] [out.png]
set -u
# In a capped scope of its own (`capped.sh`), what it starts with it: a kill stays there.
grep -q '/run-[^/]*\.scope$' /proc/self/cgroup || exec "$(dirname "$0")/capped.sh" "$0" "$@"
repo=$1; script=${2:-}; out=${3:-}
root=${GITVOLT_XDG:-$(pwd)/target/bench/work/xdg}
# Only a folder of ours, by its whole path: a joined path would land in the repository as files.
case "$root" in
  "$(pwd)"/target/bench/work/*) case "$root" in *//*) echo "run.sh: GITVOLT_XDG isn't a clean path: $root" >&2; exit 2 ;; esac ;;
  *) echo "run.sh: GITVOLT_XDG must be absolute and under target/bench/work: $root" >&2; exit 2 ;;
esac
ulimit -v "${GITVOLT_RUN_VM:-16000000}"
ulimit -c 0
mkdir -p "$root/config" "$root/data" "$root/cache"
export XDG_CONFIG_HOME=$root/config XDG_DATA_HOME=$root/data XDG_CACHE_HOME=$root/cache GITVOLT_NO_AVATARS=1
# Never the real keyring or a real forge: the fake forge when one's named, else none (`GITVOLT_TEST_FORGE`, main.rs).
export GITVOLT_TEST_FORGE=${GITVOLT_TEST_FORGE:-off}
export GIT_CONFIG_GLOBAL=${GIT_CONFIG_GLOBAL:-/dev/null} GIT_CONFIG_NOSYSTEM=1
bin=${GITVOLT_GPUI_BIN:-target/bench/release/gitvolt-gpui}
env -u WAYLAND_DISPLAY -u GIT_ASKPASS DISPLAY="${DISPLAY:-:0}" GITVOLT_SCRIPT="$script" GITVOLT_OPEN="$repo" timeout "${GITVOLT_RUN_SECS:-120}" "$bin" &
pid=$!
if [ -n "$out" ]; then
  sleep "${SHOT_WAIT:-4}"
  win=
  for id in $(xprop -root _NET_CLIENT_LIST 2>/dev/null | grep -o '0x[0-9a-f]*'); do
    if [ "$(xprop -id "$id" _NET_WM_PID 2>/dev/null | grep -o '[0-9]*$')" = "$pid" ]; then win=$id; fi
  done
  if [ -n "$win" ]; then import -window "$win" "$out"; else echo "no window for pid $pid" >&2; fi
  kill -TERM "$pid" 2>/dev/null
fi
wait "$pid" 2>/dev/null
rm -f core.* "${XDG_RUNTIME_DIR:-/nonexistent}/gitvolt-askpass-$pid-"*.sock
