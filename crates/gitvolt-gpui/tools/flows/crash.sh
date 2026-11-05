#!/usr/bin/env bash
# native: yes
# What can go away under the app, and being handled roughly: any panic in the log fails it (lib.sh's finish).
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo crash-a); B=$(small_repo crash-b); C=$(small_repo crash-c)
BIG=$WORK/flows/crash-big; rm -rf "$BIG"; mkdir -p "$BIG"; g "$BIG" init -q -b main; g "$BIG" config user.name 'Ada Fixture'; g "$BIG" config user.email ada@example.com
python3 - <<'PY' | g "$BIG" fast-import --quiet
import sys
out = sys.stdout
for i in range(4000):
    m = f"Commit {i} of the big fixture"
    out.write(f"commit refs/heads/{'main' if i % 7 else 'main'}\nmark :{i + 1}\ncommitter Ada Fixture <ada@example.com> {1700000000 + i * 600} +0000\ndata {len(m)}\n{m}\n")
    if i:
        out.write(f"from :{i}\n")
    body = f"file {i}\n"
    out.write(f"M 100644 inline d/{i % 200}.txt\ndata {len(body)}\n{body}\n\n")
    if i % 180 == 90:
        out.write(f"reset refs/heads/b{i}\nfrom :{i + 1}\n\n")
PY
g "$BIG" checkout -q main
echo "work" >>"$A/src/main.txt"; echo "gone soon" >"$A/gone.txt"
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5; pause 0.3
echo "a file deleted mid-view"
clickxy 700 147; until_q "d['wip'] and d['wip']['unstaged']" 5
clickon "tip:gone.txt"; until_q "d['open'] == 'gone.txt'" 5; pause 0.3
rm "$A/gone.txt"; pause 1.5
q "d['open'], d['wip'], d['toast']"
shot crash-gone
check "alive and honest: the file's left the list" "'gone.txt' not in d['wip']['unstaged']"
echo "a file edited, then deleted, then saved"
clickon "tip:src/main.txt" 2>/dev/null || { clickon "tip:src"; pause 0.2; clickon "tip:src/main.txt"; }; until_q "d['open'] == 'src/main.txt'" 5; pause 0.3
clickxy 700 500; key End; ri type "edit"; pause 0.2; rm "$A/src/main.txt"; pause 1.2; key Control+s; pause 1
q "d['toast'], d['popover'], d['open']"; shot crash-saved-gone
key Escape; pause 0.3; [ "$(q "d['popover']")" = True ] && key Escape; g "$A" checkout -q -- src/main.txt; pause 0.5
echo "quick clicks: the same row, rows, tabs"
launch_app "$B"; pause 0.8; launch_app "$C"; until_q "len(d['tabs']) == 3" 6; pause 0.3
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do ri key Control+$((i % 3 + 1)); done; pause 0.5
check "alive after tab switching" "len(d['tabs']) == 3"
key Control+1; pause 0.4; P=$(rowat "Change 3"); for i in 1 2 3 4 5 6 7 8 9 10; do ri move $P -- click -- key Down -- key Up; done; pause 0.5
for i in 1 2 3 4 5 6 7 8; do ri key Control+p -- key Escape -- key Control+comma -- key Escape -- key Control+slash -- key Escape; done; pause 0.5
check "alive after opening and closing things" "len(d['modals']) == 0 and not d['settings']['open']"
for i in 1 2 3 4 5 6; do clickon tb-undo-caret; key Escape; clickon hamburger; key Escape; done; pause 0.3
echo "switching tabs while something loads"
launch_app "$BIG"; for i in 1 2 3 4 5 6 7 8; do ri key Control+1; ri key Control+4; done; until_q "len(d['tabs']) == 4" 8; pause 1
echo "a repository deleted while open"
key Control+4; pause 0.6; q "d['tabs'][3], d['head']"
rm -rf "$C"; pause 2.5
q "d['toast'], d['rows']"
clickxy 700 200; pause 0.5; key Control+l; pause 1; q "d['toast'], d['running']"
echo more >/dev/null; clickon tb-branch; pause 0.4; key Escape; pause 0.3
shot crash-repo-gone
check "alive" "len(d['tabs']) >= 3"
echo "the big one under the wheel"
key Control+2; pause 2.5; q "d['rows'], d['tabs']"
ri move 700 500 -- wait 60; for i in 1 2 3 4 5 6 7 8 9 10; do ri wheel down 30; done; pause 0.3
[ -n "${REALINPUT_NATIVE:-}" ] || check "it scrolled" "d['listOffset'] < -2000"
shot crash-big
ri move 700 500; for i in 1 2 3 4 5; do ri wheel up 40; ri wheel down 50; done; pause 0.3
echo "resized very small"
if [ -z "${REALINPUT_NATIVE:-}" ]; then
  W=$(q "round(d['window']['w'])"); H=$(q "round(d['window']['h'])")
  ri move $((W - 1)) $((H - 1)) -- wait 80 -- down -- wait 60 -- move 500 300 -- wait 80 -- move 120 90 -- wait 200 -- up -- wait 500
  q "d['window']['w'], d['window']['h']"
  shot crash-small 0 0 700 500
  check "at its minimum, and alive" "d['window']['w'] == 900 and d['window']['h'] == 500"
fi
finish
