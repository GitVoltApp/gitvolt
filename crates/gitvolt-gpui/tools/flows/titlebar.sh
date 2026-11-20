#!/usr/bin/env bash
# native: yes
# Settings › Appearance › Title bar across real restarts, with the isolated instance's own binary; the button groups by keys; what moves the window
# under Combined; Restart with unsaved edits; an X11 session.
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo titlebar)
x11() { [ -z "${REALINPUT_NATIVE:-}" ]; }
appearance() { key Control+comma; pause 0.4; clickon settings-nav:appearance; pause 0.3; }
# Presses on an anchor, moves past the threshold, releases; on Xwayland, the window's left moved.
press_move() { local p; p=$(at "$1"); set -- $p; ri move "$1" "$2" -- wait 40 -- down -- wait 40 -- move $(($1 + 20)) "$2" -- wait 40 -- move $(($1 + 40)) "$2" -- wait 120 -- up -- wait 300; ri move 1 500; pause 0.2; }
win_x() { q "d['window']['x']"; }

up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5
echo "Separate, the default"
check "its row, the buttons in it" "d['chrome']['choice'] == 'separate' and d['anchors']['win-close'][1] < 30 and 'tab-bar-drag' not in d['anchors']"
appearance
shot tb-separate
check "the group's there, Separate chosen, no Restart" "d['chrome']['stored'] == 'separate' and 'setting:titleBar:combined' in d['anchors'] and 'setting:titleBarRestart' not in d['anchors']"
check "Window comes before Fonts" "d['anchors']['setting-row:titleBar'][1] < d['anchors']['setting-row:uiFontFamily'][1]"

echo "the keys of a group"
clickon setting:density:standard; pause 0.2
key Right; pause 0.2; check "Right: Comfortable" "d['chrome']['density'] == 'comfortable'"
key Right; pause 0.2; check "Right wraps: Compact" "d['chrome']['density'] == 'compact'"
key Left; pause 0.2; check "Left wraps back" "d['chrome']['density'] == 'comfortable'"
key Home; pause 0.2; check "Home" "d['chrome']['density'] == 'compact'"
key End; pause 0.2; check "End" "d['chrome']['density'] == 'comfortable'"
shot tb-density-ring
clickon setting:density:standard; pause 0.2
check "a click chooses" "d['chrome']['density'] == 'standard'"
echo "Tab reaches the next group, on what's chosen"
clickon setting:density:standard; key Tab; pause 0.2; key Right; pause 0.2
check "Right there is the title bar's" "d['chrome']['stored'] == 'combined' and d['chrome']['density'] == 'standard'"
key Left; pause 0.2
check "back: no Restart" "d['chrome']['stored'] == 'separate'"

echo "Combined offers a restart, Separate takes it back, nothing moves"
y0=$(q "d['anchors']['setting:titleBar:combined'][1]"); f0=$(q "d['anchors']['setting-row:uiFontFamily'][1]")
clickon setting:titleBar:combined; pause 0.4
check "stored, the window as it was" "d['chrome']['stored'] == 'combined' and d['chrome']['choice'] == 'separate'"
check "Restart GitVolt offered" "'setting:titleBarRestart' in d['anchors']"
check "nothing moved" "d['anchors']['setting:titleBar:combined'][1] == $y0 and d['anchors']['setting-row:uiFontFamily'][1] == $f0"
shot tb-offered
clickon setting:titleBar:separate; pause 0.3
check "back: no Restart" "'setting:titleBarRestart' not in d['anchors']"
clickon setting:titleBar:combined; pause 0.3

echo "Restart with unsaved edits asks"
key Escape; pause 0.3
echo "work" >>"$A/src/main.txt"
until_q "d['rows'] and d['rows'] >= 7" 5
clickxy 700 147; until_q "d['wip'] and d['wip']['unstaged']" 5
clickon "tip:src/main.txt" || { clickon "tip:src"; pause 0.3; clickon "tip:src/main.txt"; }
until_q "d['open'] == 'src/main.txt'" 5; pause 0.3
clickxy 800 500; pause 0.2; key End; ri type "UNSAVED"; pause 0.3
appearance
clickon setting:titleBarRestart; pause 0.5
check "it asks; the app's here" "d['popover'] and 'popover:discard' in d['anchors']"
shot tb-ask
key Escape; pause 0.3
check "Esc stays, the same process" "not d['popover'] and d['pid'] == $APP_PID"
clickon setting:titleBarRestart; pause 0.4
clickon popover:discard; pause 0.3; [ "$(q "d['popover']")" = True ] && clickon popover:discard
restarted
check_sh "it exited 0" "[ $RESTART_RC = 0 ]"
check_sh "the new one's this build, in the isolated folders" "[ \"\$(readlink /proc/$APP_PID/exe)\" = \"\$(realpath $REALINPUT_ROOT/../../../release/gitvolt-gpui)\" ] && tr '\\0' '\\n' </proc/$APP_PID/environ | grep -qx 'XDG_CONFIG_HOME=$REALINPUT_ROOT/app/config'"
check_sh "the edit wasn't written" "! grep -q UNSAVED '$A/src/main.txt'"
g "$A" checkout -q -- src/main.txt
until_q "d['rows'] and d['rows'] >= 6" 8

echo "Combined, after the restart"
check "the choice stayed, and took effect" "d['chrome']['choice'] == 'combined' and d['chrome']['stored'] == 'combined'"
check "the tab came back" "'titlebar' in d['tabs']"
check "the window buttons are in the tab bar" "d['anchors']['win-close'][1] < 34 and d['anchors']['win-close'][0] > d['anchors']['profile-switcher'][0]"
check "☰ too, after the space, and 28 px wide buttons" "d['anchors']['hamburger'][0] == 15 and d['anchors']['hamburger'][2] == 28 and d['anchors']['bell'][2] == 28 and d['anchors']['tb-settings'][2] == 28"
check "48 px of space before the bell" "d['anchors']['tab-bar-drag'][2] == 48 and d['anchors']['tab-bar-drag'][0] + 48 == d['anchors']['bell'][0]"
check "30 px more for the repository: the graph's first row is that much higher" "d['anchors']['tb-settings'][1] == 0"
shot tb-combined
appearance; shot tb-combined-settings
check "Combined chosen, no Restart now" "d['chrome']['stored'] == 'combined' and 'setting:titleBarRestart' not in d['anchors']"
key Escape; pause 0.3

echo "what moves the window, what a click still does"
for what in tab-bar-drag tab-bar-drag-lead tb-open bell tb-settings win-minimize; do
  x0=$(win_x); press_move "$what"
  check "$what: nothing opened or minimized" "not d['menuRows'] and not d['settings']['open'] and d['window']['visible'] and len(d['tabs']) == 1"
  if x11; then
    x1=$(win_x); check_sh "$what moved the window ($x0 to $x1)" "[ '$x0' != '$x1' ]"
  else
    # On Wayland a client can't say where it is: from the corner again, where a new window is placed.
    stop_app; up; until_q "d['rows'] and d['rows'] >= 6" 8
  fi
done
clickon bell; pause 0.3
check "a click on the bell opens its menu" "d['menuRows']"
clickon bell; pause 0.3
check "a second click closes it" "not d['menuRows']"
clickon tb-settings; pause 0.4
check "a click on Settings opens them" "d['settings']['open']"
key Escape; pause 0.3
clickon tb-open; pause 0.4
check "a click on Open repository opens a tab" "d['openScreen'] is not None"
n=$(q "len(d['tabs'])")
echo "the strip's double click is a title bar's; maximized, the space before ☰ goes"
S=$(q "(lambda b: str(round(b[0]+b[2]-20))+' '+str(round(b[1]+17)))(d['anchors']['strip-view'])"); set -- $S
ri move "$1" "$2" -- wait 40 -- click -- click; pause 0.8
check "maximized, no new tab" "d['window']['maximized'] and len(d['tabs']) == $n"
check "the space before ☰ is gone" "'tab-bar-drag-lead' not in d['anchors'] and d['anchors']['hamburger'][0] == 0"
shot tb-maximized
clickon win-maximize; pause 0.8
check "restored by its button" "not d['window']['maximized'] and 'tab-bar-drag-lead' in d['anchors']"

echo "a full strip scrolls: the space stays"
stop_app
tabs() { python3 - "$REALINPUT_ROOT/app/config/gitvolt-gpui/profiles/default/profile.json" "$A" "$1" <<'PY'
import json, sys
p, repo, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
v = json.load(open(p)); v["tabs"] = [{"id": f"t{i}", "kind": "repo", "path": repo, "alias": f"tab {i}"} for i in range(n)]; v["activeTab"] = "t0"; json.dump(v, open(p, "w"))
PY
}
tabs 30
up; until_q "len(d['tabs']) == 30" 10; pause 0.5
check "the strip stops at the space, the rest in place" "(lambda a: a['strip-view'][0] + a['strip-view'][2] == a['tab-bar-drag'][0] and a['tab-bar-drag'][2] == 48 and a['tab-bar-drag'][0] + 48 == a['bell'][0] and a['win-close'][0] + a['win-close'][2] <= d['window']['viewW'])(d['anchors'])"
check "a tab at its narrower minimum" "d['anchors']['tabx:0'][2] >= 91 and d['anchors']['tabx:0'][2] < 109"
shot tb-strip
if x11; then x0=$(win_x); press_move tab-bar-drag; x1=$(win_x); check_sh "it still moves the window ($x0 to $x1)" "[ '$x0' != '$x1' ]"; fi
stop_app
tabs 1
up; until_q "d['rows'] and d['rows'] >= 6" 8; pause 0.3

echo "back to Separate over a restart; then System"
appearance; clickon setting:titleBar:separate; pause 0.3
clickon setting:titleBarRestart; pause 0.3
restarted
until_q "d['chrome']['choice'] == 'separate'" 8
check "Separate again: its row" "d['anchors']['win-close'][1] < 30 and 'tab-bar-drag' not in d['anchors'] and d['anchors']['bell'][2] == 34"
appearance; clickon setting:titleBar:system; pause 0.3
clickon setting:titleBarRestart; pause 0.3
restarted
until_q "d['chrome']['choice'] == 'system'" 8
check "System: no buttons of its own, ☰ in the tab bar" "'win-close' not in d['anchors'] and 'hamburger' in d['anchors'] and 'tab-bar-drag' not in d['anchors']"
shot tb-system
if x11; then
  x0=$(win_x); press_move tb-settings
  check "with the desktop's title bar the buttons move nothing" "d['window']['x'] == $x0"
fi
# On Wayland the desktop's title bar is above what the app calls (0,0), by a height it can't ask for.
if x11; then
  clickon bell; pause 0.3; check "a click on the bell opens it there too" "d['menuRows']"; key Escape; pause 0.2
  appearance; shot tb-system-settings
  check "System chosen" "d['chrome']['stored'] == 'system'"
  clickon setting-reset:titleBar; pause 0.3
  check "its restore goes back to Separate, and offers the restart" "d['chrome']['stored'] == 'separate' and 'setting:titleBarRestart' in d['anchors']"
fi
stop_app

if x11; then
  echo "an X11 session: System, fixed"
  python3 - "$REALINPUT_ROOT/app/config/gitvolt-gpui/settings.json" <<'PY'
import json, sys
p = sys.argv[1]; v = json.load(open(p)); v["systemTitleBar"] = False; v["compactTitleBar"] = True; json.dump(v, open(p, "w"))
PY
  APP_SESSION_TYPE=x11 up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5
  check "the desktop's title bar whatever's stored" "d['chrome']['systemOnly'] and d['chrome']['choice'] == 'system' and 'win-close' not in d['anchors']"
  appearance; shot tb-x11
  clickon setting:titleBar:separate; pause 0.3
  check "a press changes nothing" "d['chrome']['stored'] == 'combined'"
  check "nothing to restart" "'setting:titleBarRestart' not in d['anchors']"
  stop_app
fi
finish
