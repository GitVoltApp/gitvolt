#!/usr/bin/env bash
# native: yes
# First start, start with a path, tabs restored, a later launch handing over its path, closing with unsaved edits, size kept.
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo life-a); B=$(small_repo life-b); C=$(small_repo life-c)
echo "first start, no config: the Open screen"
up
until_q "d['openScreen'] is not None" 5
check "one tab, the Open tab" "len(d['tabs']) == 1 and d['openScreen'] is not None"
shot life-open
stop_app
echo "start with a path"
up "$A"
until_q "d['rows'] and d['rows'] > 3" 5
check "the repository's graph" "d['head'] == 'main'"
pause 1
stop_app
echo "tabs restored"
up
until_q "d['rows'] and d['rows'] > 3" 5
check "the tab came back" "'life-a' in d['tabs']"
echo "a later launch hands its path over"
launch_app "$B"; rc=$?
check_sh "the later launch exits 0" "[ $rc = 0 ]"
until_q "'life-b' in d['tabs']" 5
check "two tabs, on the new one" "d['tabs'][-1] == 'life-b' and d['profile']['active'] is not None"
check "the first process is still the one" "d['pid'] == $APP_PID"
launch_app; rc=$?
check_sh "a later launch with no path exits 0" "[ $rc = 0 ]"

echo "a normal quit by the window's button; the size of a resized window is kept"
clickon win-close
for i in $(seq 1 50); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 0.1; done
check_sh "the close button ended it" "! kill -0 $APP_PID"
check_sh "no socket or lock's holder left" "! ls $REALINPUT_RUN/gitvolt-instance-*.sock"
kill -0 "$APP_PID" 2>/dev/null && stop_app
wait "$APP_PID" 2>/dev/null; rc=$?
check_sh "it exited 0" "[ $rc = 0 ]"

echo "closing with unsaved edits"
edit_and_close() {
  echo "work in progress" >>"$A/src/main.txt"
  until_q "any(r['summary'] == '' or True for r in d['graphRows']) and d['rows'] and d['rows'] >= 7" 5
  clickxy 700 147; until_q "d['wip'] and d['wip']['unstaged']" 5
  clickon "tip:src/main.txt" || { clickon "tip:src"; pause 0.3; clickon "tip:src/main.txt"; }
  until_q "d['open'] == 'src/main.txt'" 5
  pause 0.3
  clickxy 800 500; pause 0.2
  key End; ri type "EDITED$1"; pause 0.3
  clickon win-close; pause 0.5
}
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5
edit_and_close 1
check "the question's up, the app's alive" "d['popover']"
shot life-unsaved
clickon "popover:save"
for i in $(seq 1 50); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 0.1; done
check_sh "Save closed it" "! kill -0 $APP_PID"
check_sh "and wrote the file" "grep -q EDITED1 $A/src/main.txt"
g "$A" checkout -q -- src/main.txt

up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5
edit_and_close 2
check "the question's up again" "d['popover']"
key Escape; pause 0.3
check "Esc keeps the window, the edit there" "not d['popover'] and d['open'] == 'src/main.txt'"
clickon win-close; pause 0.5
clickon "popover:discard"
pause 0.3
clickon "popover:discard" 2>/dev/null
for i in $(seq 1 50); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 0.1; done
check_sh "Discard closed it" "! kill -0 $APP_PID"
check_sh "and didn't write" "! grep -q EDITED2 $A/src/main.txt"
g "$A" checkout -q -- src/main.txt

echo "the window's size is kept, by its edge and over a restart"
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5
if [ -z "${REALINPUT_NATIVE:-}" ]; then
  # Xwayland: the app's own client-side edge asks the window manager to resize.
  w0=$(q "d['window']['w']")
  ri move 1599 450 -- wait 80 -- down -- wait 50 -- move 1400 450 -- wait 80 -- move 1300 450 -- wait 150 -- up -- wait 300
  check "the right edge dragged narrower" "d['window']['w'] < 1450"
fi
clickon win-maximize; pause 0.8
check "the maximize button maximized" "d['window']['maximized']"
clickon win-maximize; pause 0.8
check "and restored" "not d['window']['maximized']"
W=$(q "round(d['window']['w'])"); H=$(q "round(d['window']['h'])")
clickon win-close
for i in $(seq 1 50); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 0.1; done
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5
check "the size came back" "round(d['window']['w']) == $W and round(d['window']['h']) == $H"
stop_app
finish
