#!/usr/bin/env bash
# A nested, invisible KWin with its own Xwayland, the build on a fixture inside it, and a shell line run
# against it: real events through the compositor, through the platform's own code, without touching the desktop.
# NATIVE=1 runs the app on the nested compositor's Wayland socket instead of its Xwayland (the desktop's default).
# LAYOUT=de gives the compositor that keyboard layout. A repo of `-` starts no app: the command calls `start_app`.
# usage: realinput.sh <repo|-> <command...>   (the command sees DISPLAY, GITVOLT_NESTED_SOCKET, APP_PID and these functions:
# `start_app [path]`, `stop_app`, with `tools/flows/lib.sh` on top; `tools/realinput.py` sends the input, `tools/wshot.py out.png`
# takes the picture, on Xwayland and on Wayland.)
#
# What the app can reach is cut off, so a click can't reach the desktop session: no D-Bus (no portal, no keyring), its own HOME and
# XDG folders, a login shell that's plain sh, and a PATH of an allowlist, where xdg-open and the like only write to `$OPENED`.
set -u
# Under memory pressure this and what it starts die first, not the user's programs: the kernel's pick, inherited by children.
choom -n 1000 -p $$ >/dev/null
repo=$1; shift
root=${REALINPUT_ROOT:-$(pwd)/target/bench/work/tmp/realinput}
# A socket's path has a short limit, so the runtime folder is short too.
run=${XDG_RUNTIME_DIR:-/tmp}/gvn-$$
kwin_xdg=$root/kwin
app_xdg=$root/app
mkdir -p "$kwin_xdg/config" "$kwin_xdg/data" "$kwin_xdg/cache" "$run"
chmod 700 "$run"
sock=gitvolt-nested-$$
log=$root/kwin.log
export OPENED=$root/opened.log
: >"$OPENED"
# Every window at the corner, so what the app calls (0,0) is the compositor's; no focus-stealing rules between launches.
cat >"$kwin_xdg/config/kwinrc" <<CFG
[Windows]
Placement=ZeroCornered
FocusStealingPreventionLevel=0
[Xwayland]
Scale=1
CFG
# Named, or KWin asks the system's locale settings for one.
printf '[Layout]\nUse=true\nLayoutList=%s\n' "${LAYOUT:-us}" >"$kwin_xdg/config/kxkbrc"
# A bus of the nested compositor's own, for its screenshots (`wshot.py`) only; no service can be activated on it.
cat >"$root/nested-bus.conf" <<CFG
<busconfig><type>session</type><listen>unix:path=$run/bus</listen>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/><allow user="*"/></policy></busconfig>
CFG
dbus-daemon --config-file="$root/nested-bus.conf" --nofork >/dev/null 2>&1 &
bus=$!
export GITVOLT_NESTED_BUS=unix:path=$run/bus
for _ in $(seq 1 50); do [ -S "$run/bus" ] && break; sleep 0.05; done
before=$(ls /tmp/.X11-unix 2>/dev/null)
# Its own config and session: KWin must not read the desktop's, or take over anything of the session.
env -u DISPLAY -u WAYLAND_DISPLAY -u SESSION_MANAGER \
  XDG_CONFIG_HOME="$kwin_xdg/config" XDG_DATA_HOME="$kwin_xdg/data" XDG_CACHE_HOME="$kwin_xdg/cache" XDG_RUNTIME_DIR="$run" \
  DBUS_SESSION_BUS_ADDRESS="$GITVOLT_NESTED_BUS" KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1 \
  kwin_wayland --virtual --no-lockscreen --no-global-shortcuts --no-kactivities --xwayland \
    --width "${NESTED_W:-1920}" --height "${NESTED_H:-1200}" --socket "$sock" >"$log" 2>&1 &
kwin=$!
export APP_PID=
pidfile=$root/app.pid
: >"$pidfile"
cleanup() {
  [ -s "$pidfile" ] && kill -9 "$(cat "$pidfile")" 2>/dev/null
  [ -n "${device:-}" ] && kill "$device" 2>/dev/null
  kill "$kwin" "$bus" 2>/dev/null
  wait 2>/dev/null
  rm -rf "${run:?}"
  rm -f core.*
}
trap cleanup EXIT
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
export GITVOLT_DEBUG_PIPE=$run/debug
export REALINPUT_ROOT=$root REALINPUT_RUN=$run REALINPUT_SOCK=$sock REALINPUT_PIDFILE=$pidfile REALINPUT_NATIVE=${NATIVE:-}

# The folder of what the app can run: an allowlist, and fakes that write what they were asked.
fakebin=$root/bin
rm -rf "$fakebin"; mkdir -p "$fakebin"
for t in git git-lfs sh bash dash env cat sed awk grep ls mkdir rm cp mv touch printf sleep date dirname basename uname id ssh ssh-keygen gpg fc-match tr cut head tail sort xargs true false test which wc tee find; do
  p=$(command -v "$t" || true); [ -n "$p" ] && ln -s "$p" "$fakebin/$t"
done
for t in xdg-open gio code dolphin konsole; do
  printf '#!/bin/sh\nprintf "%%s %%s\\n" "%s" "$*" >>"%s"\n' "$t" "$OPENED" >"$fakebin/$t"; chmod +x "$fakebin/$t"
done
printf '#!/bin/sh\nexit 1\n' >"$fakebin/xdg-mime"; chmod +x "$fakebin/xdg-mime"
[ -n "${FAKE_GIT:-}" ] && { rm -f "$fakebin/git"; ln -s "$FAKE_GIT" "$fakebin/git"; }

# The app, once; `start_app [path]` again is a later launch. A script of its own that execs: its pid is the app's.
cat >"$root/launch-app" <<'LAUNCH'
#!/usr/bin/env bash
ulimit -v "${GITVOLT_RUN_KB:-16000000}"; ulimit -c 0
if [ -n "${REALINPUT_NATIVE:-}" ]; then unset DISPLAY; export WAYLAND_DISPLAY=$REALINPUT_SOCK; else unset WAYLAND_DISPLAY; fi
home=$REALINPUT_ROOT/app/home
mkdir -p "$home" "$REALINPUT_ROOT/app/config" "$REALINPUT_ROOT/app/data" "$REALINPUT_ROOT/app/cache" "$REALINPUT_ROOT/app/share"
printf 'export PATH=%s\n' "$REALINPUT_ROOT/bin" >"$home/.profile"
[ -p "$GITVOLT_DEBUG_PIPE" ] || mkfifo "$GITVOLT_DEBUG_PIPE"
# The Vulkan loader finds its drivers through XDG_DATA_DIRS, which is empty here so no desktop file is found.
icds=$(ls /usr/share/vulkan/icd.d/*.json /etc/vulkan/icd.d/*.json 2>/dev/null | paste -sd:)
bin=$(realpath "${GITVOLT_GPUI_BIN:-$REALINPUT_ROOT/../../../release/gitvolt-gpui}")
# Never this repository as the working directory: a relative path typed in a field lands in the isolated home.
cd "$home" || exit 1
exec env -u GIT_ASKPASS -u SSH_AUTH_SOCK -u SSH_ASKPASS -u DESKTOP_SESSION -u SESSION_MANAGER HOME="$home" SHELL=/bin/sh PATH="$REALINPUT_ROOT/bin" \
  XDG_CONFIG_HOME="$REALINPUT_ROOT/app/config" XDG_DATA_HOME="$REALINPUT_ROOT/app/data" XDG_CACHE_HOME="$REALINPUT_ROOT/app/cache" \
  XDG_DATA_DIRS="$REALINPUT_ROOT/app/share" XDG_RUNTIME_DIR="$REALINPUT_RUN" DBUS_SESSION_BUS_ADDRESS=disabled: \
  GITVOLT_NO_AVATARS=1 GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 RUST_BACKTRACE=1 GITVOLT_TEST_FORGE="${GITVOLT_TEST_FORGE:-off}" VK_DRIVER_FILES="$icds" \
  "$bin" "$@"
LAUNCH
chmod +x "$root/launch-app"
launch_app() { "$REALINPUT_ROOT/launch-app" "$@" >>"$REALINPUT_ROOT/app.log" 2>&1; }
start_app() {
  "$REALINPUT_ROOT/launch-app" "$@" >>"$REALINPUT_ROOT/app.log" 2>&1 &
  APP_PID=$!
  echo "$APP_PID" >"$REALINPUT_PIDFILE"
  export APP_PID
}
# The way a crash ends it; a normal quit is the flow's.
stop_app() {
  [ -n "${APP_PID:-}" ] || return 0
  kill -9 "$APP_PID" 2>/dev/null
  while kill -0 "$APP_PID" 2>/dev/null; do sleep 0.05; [ -d "/proc/$APP_PID" ] && grep -q '^State:.*Z' "/proc/$APP_PID/status" && break; done
  wait "$APP_PID" 2>/dev/null
  APP_PID=
  : >"$REALINPUT_PIDFILE"
}
export -f launch_app start_app stop_app
python3 "$(dirname "$0")/realinput.py" --serve &
device=$!
for _ in $(seq 1 100); do [ -S "$run/realinput.ctl" ] && break; sleep 0.05; done
: >"$root/app.log"
if [ "$repo" != - ]; then
  start_app "$repo"
  sleep "${APP_WAIT:-4}"
fi
"$@"
