#!/usr/bin/env bash
# The build on a fixture, isolated: its config, data and cache under target/bench/work/xdg, no avatars
# asked for. A script (GITVOLT_SCRIPT, bench.rs) runs its flow; a screenshot of this window comes at
# `shot <out.png>` lines' moments by `tools/shot.sh`'s way, when one's asked for with SHOT_AT.
# usage: run.sh <repo> [script] [out.png]
set -u
repo=$1; script=${2:-}; out=${3:-}
root=$(pwd)/target/bench/work/xdg
mkdir -p "$root/config" "$root/data" "$root/cache"
export XDG_CONFIG_HOME=$root/config XDG_DATA_HOME=$root/data XDG_CACHE_HOME=$root/cache GITVOLT_NO_AVATARS=1
export GIT_CONFIG_GLOBAL=${GIT_CONFIG_GLOBAL:-/dev/null} GIT_CONFIG_NOSYSTEM=1
bin=${GITVOLT_GPUI_BIN:-target/bench/release/gitvolt-gpui}
env -u WAYLAND_DISPLAY DISPLAY="${DISPLAY:-:0}" GITVOLT_SCRIPT="$script" GITVOLT_OPEN="$repo" "$bin" &
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
