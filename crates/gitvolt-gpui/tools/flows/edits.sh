#!/usr/bin/env bash
# native: yes
# Editing and its questions: a stale save, a hidden tab's unsaved close, discarding a hunk.
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo edit-a); B=$(small_repo edit-b)
echo "base of a work in progress" >>"$A/src/main.txt"
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5; pause 0.3
open_wip() { clickxy 700 147; until_q "d['wip'] and d['wip']['unstaged']" 5; clickon "tip:src/main.txt" || { clickon "tip:src"; pause 0.2; clickon "tip:src/main.txt"; }; until_q "d['open'] == 'src/main.txt'" 5; pause 0.4; }
open_wip
echo "typing, Save, undo in the editor"
clickxy 700 500; pause 0.1; key End; ri type " TYPED"; pause 0.2
check "dirty" "d['diff']['dirty']"
key Control+z; pause 0.2
check "undo in the editor takes it back" "not d['diff']['dirty']"
key Control+Shift+z; pause 0.2; check "redo" "d['diff']['dirty']"
key Control+s; pause 0.6
check "saved: clean" "not d['diff']['dirty']"
check_sh "on disk" "grep -q TYPED '$A/src/main.txt'"
echo "a stale save: Overwrite"
key End; ri type "MINE"; pause 0.2
echo "changed behind its back" >>"$A/src/main.txt"
pause 0.8
key Control+s; pause 0.6
shot edits-stale
check "Stale asks" "d['popover'] or d['toast']"
q "sorted(k for k in d['anchors'] if k.startswith('popover:'))"
clickon "popover:overwrite"; pause 0.3
clickon "popover:overwrite"; pause 0.6
check_sh "Overwrite wrote mine" "grep -q MINE '$A/src/main.txt'"
check_sh "and dropped theirs" "! grep -q 'changed behind its back' '$A/src/main.txt'"
echo "a stale save: Reload from disk"
pause 0.5; key End; ri type "AGAIN"; pause 0.2
echo "changed again" >>"$A/src/main.txt"; pause 0.8
key Control+s; pause 0.6
clickon "popover:reload"; pause 0.8
check "Reload: clean" "not d['diff']['dirty']"
check_sh "disk kept theirs" "grep -q 'changed again' '$A/src/main.txt'"
echo "a hidden tab's unsaved close: the window asks about it, on its tab"
key End; ri type "HIDDEN"; pause 0.2
launch_app "$B"; until_q "len(d['tabs']) == 2 and d['rows'] == 6" 5; pause 0.3
check "on the other tab" "d['open'] is None"
clickon win-close; pause 0.6
check "it asks, where it is, as the web's" "d['popover'] and d['open'] is None"
shot edits-hidden
key Escape; pause 0.3
echo "closing that tab itself asks too"
clickon "tabx:0" ; pause 0.1
P=$(q "at('tip:Close (Ctrl+W)')"); echo "tab close at $P"
key Control+w; pause 0.5
check "Ctrl+W on the file asks" "d['popover']"
clickon "popover:discard"; pause 0.3; [ "$(q "d['popover']")" = True ] && clickon "popover:discard"; pause 0.5
check "the file closed, its work gone" "d['open'] is None"
check_sh "not written" "! grep -q HIDDEN '$A/src/main.txt'"
echo "Discard hunk"
g "$A" checkout -q -- src/main.txt; pause 0.5
for i in 50 150; do sed -i "s/^line $i of/LINE $i of/" "$A/src/main.txt"; done; pause 1
open_wip
key Control+Shift+1; pause 0.4
check "Hunk shows its changes, not an empty end" "d['diff']['mode'] == 'Hunk' and d['diff']['scroll'] < d['diff']['max']"
ri move 600 300 -- wait 300
shot edits-hunks
clickxy 978 213; pause 0.9
check_sh "Discard hunk discards that hunk, undoably" "! grep -q 'LINE 50 of' '$A/src/main.txt' && grep -q 'LINE 150 of' '$A/src/main.txt'"
q "d['diff'] and d['diff']['mode'], d['wip']"
clickon tb-undo; pause 0.9
check_sh "and Undo brings it back" "grep -q 'LINE 50 of' '$A/src/main.txt'"
echo "Stage hunk"
ri move 600 300 -- wait 200; shot edits-hunk2
clickxy 1077 213; pause 0.9
q "d['wip']"
check_sh "the hunk's staged" "git -C '$A' diff --cached | grep -q 'LINE 50 of'"
finish
