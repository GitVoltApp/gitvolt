#!/usr/bin/env bash
# What the parity round added and rows the sweep hadn't run: the settings' Tab stops, Hosts, the track of a scrollbar, a theme switch
# with a source diff open, find's toggles, sticky scroll, the Colour row's strip, the graph's sideways scroll.
source "$(dirname "$0")/lib.sh"
clean_app
echo '{"version":2,"gravatar":false,"updateCheck":false,"fetchIntervalSecs":0,"editorStickyScroll":true}' >"$REALINPUT_ROOT/app/config/gitvolt-gpui/settings.json"
A=$(small_repo r2a)
for i in $(seq 1 120); do echo "    fn f$i() { let long = \"a line long enough to pass the edge of the pane when it is read in a narrow one, and on past it\"; }"; done >"$A/src/lib.rs"
{ echo "mod outer {"; echo "    impl Thing {"; for i in $(seq 1 200); do echo "        // line $i in a scope"; done; echo "    }"; echo "}"; } >"$A/src/scope.rs"
g "$A" add src; g "$A" commit -qm 'Long lines and scopes'
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5; pause 0.3
echo "settings: Tab reaches a switch, Space flips it"
key Control+comma; pause 0.5; clickon settings-nav:general; pause 0.3
before=$(q "d['settings'].get('advanceAfterStage')"); key Tab; pause 0.1; shot r2-tab1
for i in 1 2 3 4 5 6; do key Tab; done; pause 0.2; shot r2-tab2
check "still in the dialog" "d['settings']['open']"
check "the keyboard hasn't gone behind it" "not d.get('filterFocused')"
echo "Hosts: a host added, its row, its kind"
clickon settings-nav:hosts; pause 0.3; clickon host-add-box; pause 0.1; ri type "git.example.net"; pause 0.2; shot r2-hosts; clickon setting:host-add; pause 0.5; shot r2-hosts2
echo "a page's scrollbar: a press on its track pages"
clickon settings-nav:theme; pause 0.5
B=$(q "(lambda b: str(round(b['x']+b['w']+4))+' '+str(round(b['y']+b['h']-40)))(d['anchors']['settings-nav:theme'])" 2>/dev/null); shot r2-track0
ri move 1185 760 -- wait 40 -- click; pause 0.4; shot r2-track1
echo "a theme switch with a source diff open"
key Escape; pause 0.4; check "Esc closed it" "not d['settings']['open']"; clickrow "Long lines and scopes"; pause 0.6; shot r2-files
key Control+p; pause 0.3; ri type "/lib.rs"; pause 0.5; key Return; pause 1
shot r2-diff
key Control+comma; pause 0.5; clickon settings-nav:theme; pause 0.3; clickon "theme-card:light"; pause 0.8; key Escape; pause 0.8; shot r2-light
check "light, live" "d['settings']['themeShown'] == 'light'"
key Control+comma; pause 0.5; clickon settings-nav:theme; pause 0.3; clickon "theme-card:default-dark"; pause 0.5; key Escape; pause 0.6
echo "find's toggles"
key Control+f; pause 0.3; ri type "LONG"; pause 0.4; shot r2-find
clickon find-case; pause 0.3; shot r2-find-case
clickon find-case; pause 0.2; clickon find-regex; pause 0.2; key Control+a; ri type "long.*edge"; pause 0.4; shot r2-find-regex
key Escape; pause 0.3
echo "the file list's Colour row: the page's strip; Add"
clickon "tip:src" right; pause 0.3; shot r2-menu
pick "Colour"; pause 0.6; shot r2-strip
check "the page is up" "d['settings']['open'] and d['settings']['page'] == 'file-colours'"
clickon fc-strip-new; pause 0.5; shot r2-strip2
echo "a rule dragged in a preset"
clickon settings-nav:branch-presets; pause 0.4; clickon bp-new; pause 0.5; clickon bp-add-rule; pause 0.3; clickon bp-add-rule; pause 0.4; shot r2-rules
R0=$(q "(lambda b: f\"{round(b['x']+14)} {round(b['y']+b['h']/2)}\")(d['anchors']['bp-rule:0'])" 2>/dev/null); R1=$(q "(lambda b: f\"{round(b['x']+14)} {round(b['y']+b['h']/2+4)}\")(d['anchors']['bp-rule:1'])" 2>/dev/null)
[ -n "$R0" ] && [ -n "$R1" ] && { ri move $R0 -- wait 60 -- down -- wait 60 -- move $R1 -- wait 200 -- up; pause 0.4; shot r2-rules2; }
echo "a lane's colour dragged on the theme page"
clickon settings-nav:theme; pause 0.4; L0=$(at "lane:0"); L2=$(at "lane:2"); [ -n "$L0" ] && [ -n "$L2" ] && { set -- $L0; a=$1; ri move $L0 -- wait 60 -- down -- wait 60 -- move $((a + 20)) $2 -- wait 60 -- move $L2 -- wait 200 -- up; pause 0.5; shot r2-lanes; }
for _ in 1 2 3; do [ "$(q "d['settings']['open']")" = True ] && { key Escape; pause 0.4; }; done
check "Esc closed the dialog, the picker first" "not d['settings']['open']"
echo "sticky scroll, in File View"
for _ in 1 2 3; do [ "$(q "d['settings']['open']")" = True ] && { key Escape; pause 0.4; }; done
key Control+p; pause 0.3; ri type "/scope.rs"; pause 0.5; key Return; pause 1
ri move 700 500 -- wait 40 -- wheel down 6 -- wait 300; shot r2-sticky
finish
