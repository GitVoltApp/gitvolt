#!/usr/bin/env bash
# native: yes
# The command table's chords by real keys, with what each should have done.
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo keys-a); B=$(small_repo keys-b)
echo "more" >>"$A/README.md"
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5
launch_app "$B"; until_q "len(d['tabs']) == 2" 5; pause 0.5
c() { key "$1"; pause 0.35; check "$1: $2" "$3"; }
c Control+1 "first tab" "d['profile']['active'] == d['profile']['active'] and d['head'] and d['rows'] == 7"
c Control+2 "second tab" "d['rows'] == 6"
c Control+9 "the last tab" "d['rows'] == 6"
c Control+1 "first tab again" "d['rows'] == 7"
c Control+o "Open repository" "d['openScreen'] is not None"
c Control+w "Close tab: the Open tab goes" "d['openScreen'] is None"
c Control+Shift+t "nothing to reopen but the open tab, or it" "True"
[ "$(q "d['openScreen'] is not None")" = True ] && key Control+w
pause 0.3
c Control+1 "first tab" "d['rows'] == 7"
c Control+comma "Settings" "d['settings']['open']"
key Escape; pause 0.3
c Control+p "palette" "'palette' in str(d['modals'])"
key Escape; pause 0.3
c Control+slash "shortcuts" "'shortcuts' in str(d['modals'])"
key Escape; pause 0.3
c Control+b "sidebar to strip" "'side:refs/heads/main' not in d['anchors']"
c Control+b "and back" "'side:refs/heads/main' in d['anchors']"
c Control+Alt+f "the sidebar's filter" "True"
ri type "fea"; pause 0.3; shot keys-filter
key Escape; pause 0.2
c Control+f "Find in the graph" "True"
ri type "Change 3"; pause 0.4; shot keys-find
key Escape; pause 0.3
c Control+Shift+g "Go to WIP" "d['wip'] and d['selected'] == 0"
clickon "tip:README.md"; until_q "d['open'] == 'README.md'" 5; pause 0.3
c Control+Shift+3 "Split" "d['diff']['mode'] == 'Split'"
c Control+Shift+1 "Hunk" "d['diff']['mode'] == 'Hunk'"
c Control+Shift+2 "Inline" "d['diff']['mode'] == 'Inline'"
c F7 "next change" "True"
c Control+Shift+h "File history" "True"; shot keys-history
key Escape; pause 0.3
[ "$(q "d['open'] is None")" = True ] && { clickon "tip:README.md"; until_q "d['open'] == 'README.md'" 5; pause 0.3; }
c Control+Shift+b "Blame" "True"; shot keys-blame
key Escape; pause 0.3
[ "$(q "d['open'] is None")" = True ] && { clickon "tip:README.md"; until_q "d['open'] == 'README.md'" 5; pause 0.3; }
c Control+w "Close file" "d['open'] is None and len(d['tabs']) == 2"
c Control+equal "zoom in" "d['zoom'] == 110"
c Control+minus "zoom out" "d['zoom'] == 100"
c Control+plus "zoom in by +" "d['zoom'] == 110"
c Control+0 "reset" "d['zoom'] == 100"
c Alt+2 "the graph has the keyboard" "True"
key Down; pause 0.2; check "Down moves down" "d['selected'] == 1"
key Up; pause 0.2; check "Up moves up" "d['selected'] == 0"
key Down; key Down; key Down; pause 0.2
key Home; pause 0.2; check "Home" "d['selected'] == 0"
key End; pause 0.2; check "End" "d['selected'] == d['rows'] - 1"
key PageUp; pause 0.2; check "PageUp" "d['selected'] < d['rows'] - 1"
key PageDown; pause 0.2; check "PageDown" "d['selected'] == d['rows'] - 1"
key Home; key Shift+Down; key Shift+Down; pause 0.2; check "Shift+Down extends" "len(d['picks']) == 3"
key Escape; pause 0.2
key Home; pause 0.2
c Control+Shift+n "Branch: the inline field" "d['editor'] is not None or len(d['modals']) == 1"
ri type "from-keys"; key Return; pause 0.8
check_sh "the branch was made" "git -C '$A' rev-parse -q --verify refs/heads/from-keys"
c Control+l "Fetch" "True"
c Control+z "Undo the branch" "True"; pause 0.5
check_sh "undone" "! git -C '$A' rev-parse -q --verify refs/heads/from-keys"
c Control+Shift+z "Redo" "True"; pause 0.5
check_sh "redone" "git -C '$A' rev-parse -q --verify refs/heads/from-keys"
c Control+Alt+s "Stash" "True"; pause 0.8
check_sh "stashed" "git -C '$A' stash list | grep -q ."
c Control+Alt+p "Pop" "True"; pause 0.8
check_sh "popped" "! git -C '$A' stash list | grep -q ."
c Control+Alt+g "Next preset" "True"
c Control+Alt+Shift+g "Choose preset" "len(d['menu']) > 0 or d['picker']"
key Escape; pause 0.2
key Control+Alt+g; pause 0.3; key Control+Alt+g; pause 0.3; key Control+Alt+g; pause 0.3
echo "commit by Ctrl+Enter from the box"
key Control+Shift+g; pause 0.3
clickxy 1400 708; pause 0.2; ri type "Commit from keys"; key Control+Shift+s; pause 0.1
clickon "tip:README.md"; pause 0.4
c Control+Shift+s "Stage file" "d['wip'] and d['wip']['staged'] == ['README.md']"
key Control+w; pause 0.3
clickxy 1400 708; pause 0.2
c Control+Return "Commit" "True"; pause 1
check_sh "committed" "git -C '$A' log -1 --format=%s | grep -q 'Commit from keys'"
finish
