#!/usr/bin/env bash
# A nested, invisible KWin with its own Xwayland, the build on a fixture inside it, and a shell line run
# against it: real events from an X server, through the platform's own code, without touching the desktop.
# NATIVE=1 runs the app on the nested compositor's Wayland socket instead of its Xwayland (the desktop's default);
# no `xshot.sh` there.
# usage: realinput.sh <repo> <command...>   (the command sees DISPLAY, GITVOLT_NESTED_SOCKET and APP_PID;
# `tools/realinput.py` sends the input, `tools/xshot.sh out.png` takes the picture)
set -u
repo=$1; shift
root=$(pwd)/target/bench/work/tmp/realinput
# A socket's path has a short limit, so the runtime folder is short too.
run=${XDG_RUNTIME_DIR:-/tmp}/gvn-$$
mkdir -p "$root/config" "$root/data" "$root/cache" "$run"
chmod 700 "$run"
sock=gitvolt-nested-$$
log=$root/kwin.log
app=
before=$(ls /tmp/.X11-unix 2>/dev/null)
# Its own config and session: KWin must not read the desktop's, or take over anything of the session.
env -u DISPLAY -u WAYLAND_DISPLAY -u SESSION_MANAGER \
  XDG_CONFIG_HOME=$root/config XDG_DATA_HOME=$root/data XDG_CACHE_HOME=$root/cache XDG_RUNTIME_DIR=$run \
  DBUS_SESSION_BUS_ADDRESS=disabled: KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 \
  kwin_wayland --virtual --no-lockscreen --no-global-shortcuts --no-kactivities --xwayland \
    --width "${NESTED_W:-1600}" --height "${NESTED_H:-900}" --socket "$sock" >"$log" 2>&1 &
kwin=$!
trap '[ -n "$app" ] && kill "$app" 2>/dev/null; kill "$kwin" 2>/dev/null; wait 2>/dev/null; rm -rf "${run:?}"' EXIT
disp=
for _ in $(seq 1 100); do
  # KWin takes the first free :N; a socket that wasn't there before is its.
  new=$(ls /tmp/.X11-unix 2>/dev/null | grep -vxF "$before" | head -1)
  [ -n "$new" ] && { disp=:${new#X}; break; }
  kill -0 "$kwin" 2>/dev/null || { echo "kwin exited:"; tail -20 "$log"; exit 1; }
  sleep 0.1
done
[ -n "$disp" ] || { echo "no nested display"; tail -20 "$log"; exit 1; }
[ "$disp" != "${DISPLAY:-}" ] || { echo "nested display equals the desktop's: refusing"; exit 1; }
export DISPLAY=$disp GITVOLT_NESTED_DISPLAY=$disp GITVOLT_NESTED_SOCKET=$run/$sock
( ulimit -v "${GITVOLT_RUN_KB:-16000000}"
  if [ -n "${NATIVE:-}" ]; then unset DISPLAY; export WAYLAND_DISPLAY=$sock; else unset WAYLAND_DISPLAY; fi
  exec env XDG_CONFIG_HOME="$root/config" XDG_DATA_HOME="$root/data" XDG_CACHE_HOME="$root/cache" \
    XDG_RUNTIME_DIR="$run" GITVOLT_NO_AVATARS=1 GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
    GITVOLT_OPEN="$repo" "${GITVOLT_GPUI_BIN:-target/bench/release/gitvolt-gpui}" ) >"$root/app.log" 2>&1 &
app=$!
export APP_PID=$app
sleep "${APP_WAIT:-4}"
"$@"
