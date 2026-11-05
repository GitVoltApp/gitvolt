#!/usr/bin/env bash
# What a flow is written in; `source` it from a script `tools/flows/run.sh` (or `tools/realinput.sh`) runs. Everything goes to the
# nested compositor by `realinput.py`, and what's checked is what the app says of itself (`GITVOLT_DEBUG_PIPE`, `bench.rs`), the
# fixture's git state or a picture.
TOOLS=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
WORK=$(pwd)/target/bench/work
SHOTS=$WORK/shots/flows
TMPD=$WORK/tmp/flows
mkdir -p "$SHOTS" "$TMPD"
FAILS=0
CHECKS=0
# In a session (`session.sh`) the pid comes from the file.
[ -z "${APP_PID:-}" ] && [ -s "${REALINPUT_PIDFILE:-/nonexistent}" ] && APP_PID=$(cat "$REALINPUT_PIDFILE")

ri() { python3 "$TOOLS/realinput.py" --send "$@"; }
# The window's own part of the screen, or `shot name x y w h`.
shot() {
  if [ $# -gt 1 ]; then python3 "$TOOLS/wshot.py" "$SHOTS/$1.png" "${@:2}"; else python3 "$TOOLS/wshot.py" "$SHOTS/$1.png" 0 0 $(q "str(round(d['window']['w'])) + ' ' + str(round(d['window']['h']))"); fi
}
g() { git -C "$1" "${@:2}"; }

# The app's state as JSON in $STATE; a dead or hung app fails the flow here.
state() {
  rm -f "$GITVOLT_DEBUG_PIPE.out"
  kill -0 "${APP_PID:-0}" 2>/dev/null && timeout 2 bash -c "echo x >\"$GITVOLT_DEBUG_PIPE\""
  for _ in $(seq 1 200); do
    [ -f "$GITVOLT_DEBUG_PIPE.out" ] && { STATE=$GITVOLT_DEBUG_PIPE.out; return 0; }
    kill -0 "${APP_PID:-0}" 2>/dev/null || { fail "the app is gone"; tail -30 "$REALINPUT_ROOT/app.log"; finish; exit 3; }
    sleep 0.02
  done
  fail "the app doesn't answer: hung"
  finish
  exit 4
}
# A Python expression over `d`, the state, with the helpers in q.py.
q() { state && python3 "$TOOLS/flows/q.py" "$1" "$STATE"; }
fail() { FAILS=$((FAILS + 1)); echo "  FAIL: $*"; }
pass() { echo "  ok: $*"; }
# check <what> <Python expression over d>
check() {
  CHECKS=$((CHECKS + 1))
  local r
  r=$(q "bool($2)" 2>&1)
  if [ "$r" = True ]; then pass "$1"; else fail "$1 [$2 -> $(printf '%s' "$r" | tail -1 | cut -c1-300)]"; fi
}
# check_sh <what> <shell command>
check_sh() {
  CHECKS=$((CHECKS + 1))
  if eval "$2" >/dev/null 2>&1; then pass "$1"; else fail "$1 [$2]"; fi
}
# Waits until an expression holds, up to a time.
until_q() {
  local expr=$1 secs=${2:-5} i
  for i in $(seq 1 $((secs * 10))); do
    [ "$(q "bool($expr)" 2>/dev/null)" = True ] && return 0
    sleep 0.1
  done
  fail "waited ${secs}s for [$expr] -> $(q "$expr" 2>&1 | tail -1 | cut -c1-300)"
  return 1
}
# An anchor's centre by the end of its key, as `x y`; a menu's or a picker's row by its text; a graph row by its summary.
at() { q "at('''$1''')"; }
row_at() { q "row_at('''$1''')"; }
rowat() { q "rowat('''$1''')"; }
clickrow() { local p; p=$(rowat "$1"); [ -n "$p" ] || { fail "no graph row $1"; return 1; }; ri move $p -- wait 40 -- click "${2:-left}"; }
moveon() { local p; p=$(at "$1"); [ -n "$p" ] || { fail "no anchor $1"; return 1; }; ri move $p; }
clickon() { local p; p=$(at "$1"); [ -n "$p" ] || { fail "no anchor $1"; return 1; }; ri move $p -- wait 40 -- click "${2:-left}"; }
dclickon() { local p; p=$(at "$1"); [ -n "$p" ] || { fail "no anchor $1"; return 1; }; ri move $p -- wait 40 -- click -- click; }
pick() { local p; p=$(row_at "$1"); [ -n "$p" ] || { fail "no row $1"; return 1; }; ri move $p -- wait 40 -- click; }
clickxy() { ri move "$1" "$2" -- wait 40 -- click "${3:-left}"; }
key() { for k; do ri key "$k"; done; }
pause() { sleep "$1"; }
# A clean start: no profile, no settings, no window size. Settings written first keep it off the network and away from what a
# previous run left.
clean_app() {
  rm -rf "$REALINPUT_ROOT/app"
  mkdir -p "$REALINPUT_ROOT/app/config/gitvolt-gpui"
  echo '{"version":2,"gravatar":false,"updateCheck":false,"fetchIntervalSecs":0}' >"$REALINPUT_ROOT/app/config/gitvolt-gpui/settings.json"
}
# Starts it and waits for its answer, with a repository's graph if one's given.
up() {
  start_app "$@"
  local i
  for i in $(seq 1 100); do
    [ -p "$GITVOLT_DEBUG_PIPE" ] && state 2>/dev/null >/dev/null && [ -f "$STATE" ] && break
    sleep 0.1
  done
  until_q "d['window']['active']" 5 || true
  # A pointer at rest in the corner, where no hover can matter.
  ri move 1 500
}
# A small repository to write in, fictional, under the work folder, made anew.
small_repo() {
  local dir=$WORK/flows/$1
  rm -rf "$dir"; mkdir -p "$dir"
  g "$dir" init -q -b main
  g "$dir" config user.name 'Ada Fixture'; g "$dir" config user.email ada@example.com
  printf '# Flow fixture\n\nSome *text*.\n' >"$dir/README.md"
  mkdir -p "$dir/src"; for i in $(seq 1 200); do echo "line $i of the source"; done >"$dir/src/main.txt"
  g "$dir" add README.md src; g "$dir" commit -qm 'Base'
  for i in 1 2 3 4 5; do echo "change $i" >>"$dir/README.md"; g "$dir" add README.md; g "$dir" commit -qm "Change $i"; done
  g "$dir" branch feature/one HEAD~2; g "$dir" branch feature/two HEAD~1; g "$dir" tag v1.0.0 HEAD~3
  echo "$dir"
}
# Ends a flow: what its log says, and the count.
finish() {
  local panics
  panics=$(grep -c "panicked at" "$REALINPUT_ROOT/app.log" 2>/dev/null); panics=${panics:-0}
  if [ "$panics" != 0 ]; then
    fail "the app panicked $panics times:"
    grep -A12 "panicked at" "$REALINPUT_ROOT/app.log" | head -40
  fi
  echo "RESULT $(basename "${FLOW_NAME:-$0}" .sh): $((CHECKS - FAILS))/$CHECKS checks, $FAILS failures"
  [ "$FAILS" = 0 ]
}
# A repository with a long graph, fast, with a file of many lines, and a change on top of it in the working copy.
long_repo() {
  local dir=$WORK/flows/$1 n=${2:-600}
  rm -rf "$dir"; mkdir -p "$dir"
  g "$dir" init -q -b main
  g "$dir" config user.name 'Ada Fixture'; g "$dir" config user.email ada@example.com
  python3 - "$n" <<'PY' | g "$dir" fast-import --quiet
import sys
n = int(sys.argv[1])
text = "".join(f"line {i} of the long file, with some words to select\n" for i in range(3000))
out = sys.stdout
mark = 0
for i in range(n):
    mark += 1
    body = text.replace(f"line {i * 3} of", f"LINE {i * 3} of") if i else text
    msg = f"Commit number {i} in the long fixture"
    out.write(f"commit refs/heads/main\nmark :{mark}\ncommitter Ada Fixture <ada@example.com> {1700000000 + i * 3600} +0000\ndata {len(msg)}\n{msg}\n")
    if mark > 1:
        out.write(f"from :{mark - 1}\n")
    out.write(f"M 100644 inline long.txt\ndata {len(body)}\n{body}\n")
    other = f"file {i}\n"
    out.write(f"M 100644 inline dir/f{i % 150}.txt\ndata {len(other)}\n{other}\n\n")
    if i % 50 == 25:
        out.write(f"reset refs/heads/topic/b{i}\nfrom :{mark}\n\n")
PY
  g "$dir" checkout -q main
  sed -i 's/^line 1000 of/LINE ONE THOUSAND of/; s/^line 2500 of/LINE TWENTY-FIVE HUNDRED of/' "$dir/long.txt"
  echo "$dir"
}
