#!/usr/bin/env bash
# native: yes
# layout: de
# A German keyboard, by physical keys (evdev codes): AltGr characters and umlauts in a field, and the shortcuts that land elsewhere there.
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo german)
echo "more" >>"$A/README.md"
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5
clickxy 700 147; until_q "d['wip'] and d['wip']['unstaged']" 5
CTRL=29; SHIFT=42; ALT=56; ALTGR=100
clickxy 1400 708; pause 0.2
ri raw $ALTGR+16 -- raw $ALTGR+9 -- raw $ALTGR+10 -- raw $ALTGR+8 -- raw $ALTGR+11 -- raw $ALTGR+12 -- raw $ALTGR+86 -- raw $ALTGR+27 -- raw $ALTGR+18
pause 0.3
check "AltGr: @ [ ] { } \\ | ~ €" "d['summary'] == '@[]{}\\\\|~€'"
q "d['summary']"
ri raw 39 -- raw 40 -- raw 26 -- raw 12 -- raw $SHIFT+39 -- raw $SHIFT+40 -- raw $SHIFT+26; pause 0.3
check "ö ä ü ß Ö Ä Ü" "d['summary'].endswith('öäüßÖÄÜ')"
q "d['summary']"
shot german-field
ri raw 21; ri raw 44; pause 0.2
check "y and z where they are on a German keyboard" "d['summary'].endswith('zy')"
key Control+a; ri raw 14; pause 0.2
check "the field's empty" "d['summary'] == ''"
echo "a drag held past the field's right edge scrolls on to the text's end"
ri type "Merge branch main into a branch with a rather long name, longer than the summary field shows at once"; pause 0.3
key Home; pause 0.2
ri move 1250 708 -- wait 60 -- down -- wait 80 -- move 1450 708 -- wait 60 -- move 1598 708 -- wait 2500 -- up; pause 0.3
check "the selection went on to the end" "d['summarySel'][1] == len(d['summary'].encode()) and d['summarySel'][0] < 20"
q "d['summarySel'], len(d['summary'])"
key Control+a; ri raw 14; pause 0.2
key Escape; pause 0.2
# The shortcuts: out of the field.
clickxy 700 500; pause 0.2
echo "shortcuts"
ri raw $CTRL+51; pause 0.5
check "Ctrl+, opens Settings" "d['settings']['open']"
ri key Escape; pause 0.4
check "Esc closes it" "not d['settings']['open']"
ri raw $CTRL+53; pause 0.3
check "Ctrl+- zooms out" "d['zoom'] == 90"
ri raw $CTRL+27; pause 0.3
check "Ctrl++ zooms in" "d['zoom'] == 100"
ri raw $CTRL+$SHIFT+11; pause 0.3
check "Ctrl+= (Ctrl+Shift+0 here) zooms in" "d['zoom'] == 110"
ri raw $CTRL+11; pause 0.3
check "Ctrl+0 resets" "d['zoom'] == 100"
ri raw $CTRL+$SHIFT+8; pause 0.5
check "Ctrl+/ (Ctrl+Shift+7 here) opens the shortcuts panel" "len(d['modals']) == 1"
q "d['modals']"
key Escape; pause 0.3
check "and Esc closes it" "len(d['modals']) == 0"
ri raw $CTRL+25; pause 0.5
check "Ctrl+P the palette" "len(d['modals']) == 1"
ri raw 40 -- raw 39; pause 0.3
shot german-palette
key Escape; pause 0.3
echo "the file's modes, Ctrl+Shift+digit"
clickon "tip:README.md"; until_q "d['open'] == 'README.md'" 5
pause 0.3
ri raw $CTRL+$SHIFT+4; pause 0.3
check "Ctrl+Shift+3 is Split" "d['diff']['mode'] == 'Split'"
ri raw $CTRL+$SHIFT+3; pause 0.3
check "Ctrl+Shift+2 is Inline" "d['diff']['mode'] == 'Inline'"
ri raw $CTRL+$SHIFT+2; pause 0.3
check "Ctrl+Shift+1 is Hunk" "d['diff']['mode'] == 'Hunk'"
q "d['input'][-3:]"
finish
