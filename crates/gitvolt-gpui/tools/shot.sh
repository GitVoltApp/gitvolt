#!/usr/bin/env bash
# A screenshot of the GPUI build's own window and nothing else: it runs under XWayland, and the
# window is found by this process's pid, so no other window on the desktop can end up in the image.
# usage: shot.sh <out.png> <repo> [GITVOLT_SHOW spec]
set -u
out=$1; repo=$2; spec=${3:-}
bin=${GITVOLT_GPUI_BIN:-target/bench/release/gitvolt-gpui}
env -u WAYLAND_DISPLAY DISPLAY="${DISPLAY:-:0}" GITVOLT_SHOW="$spec" GITVOLT_OPEN="$repo" "$bin" >/dev/null 2>&1 &
pid=$!
sleep "${SHOT_WAIT:-4}"
win=
for id in $(xprop -root _NET_CLIENT_LIST 2>/dev/null | grep -o '0x[0-9a-f]*'); do
  if [ "$(xprop -id "$id" _NET_WM_PID 2>/dev/null | grep -o '[0-9]*$')" = "$pid" ]; then win=$id; fi
done
if [ -z "$win" ]; then echo "no window for pid $pid" >&2; kill -TERM "$pid"; exit 1; fi
import -window "$win" "$out"
kill -TERM "$pid"
wait "$pid" 2>/dev/null
rm -f "${XDG_RUNTIME_DIR:-/nonexistent}/gitvolt-askpass-$pid-"*.sock
