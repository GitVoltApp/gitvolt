#!/usr/bin/env bash
# native: yes
# Selecting with a mouse in a diff, and the clipboard round trip.
source "$(dirname "$0")/lib.sh"
clean_app
L=$(long_repo select 40)
up "$L"; until_q "d['rows'] and d['rows'] > 10" 10
clickxy 700 147; until_q "d['wip'] and d['wip']['unstaged']" 5
clickon "tip:long.txt"; until_q "d['open'] == 'long.txt' and d['diff']" 5; pause 0.5
shot select-0
LX=500; LY=400
echo "a drag selects"
ri move 300 $LY -- wait 40 -- down -- wait 40 -- move 340 $LY -- wait 40 -- move 400 $LY -- wait 100 -- up -- wait 200
check "text selected" "d['diff']['selected']"
q "d['diff']['selected']"
echo "double click: a word"
ri move 350 $LY -- wait 500 -- click -- click -- wait 200
check "a word" "d['diff']['selected'] and ' ' not in d['diff']['selected'].strip()"
q "d['diff']['selected']"
echo "triple click: the line"
ri move 350 $LY -- wait 500 -- click -- click -- click -- wait 200
check "a line" "d['diff']['selected'] and 'with some words to select' in d['diff']['selected']"
q "d['diff']['selected']"
echo "shift+click extends"
ri move 500 $((LY - 40)) -- wait 500 -- click -- wait 100
ri hold Shift -- wait 30 -- move 500 $((LY + 60)) -- wait 40 -- click -- wait 30 -- release Shift -- wait 150
check "many lines" "d['diff']['selected'] and d['diff']['selected'].count(chr(10)) >= 3"
q "repr(d['diff']['selected'])[:120]"
echo "a selection, then Ctrl+C"
key Control+c; pause 0.3
check "copied" "d['clipboard'] and 'with some words' in d['clipboard']"
echo "pasted into the commit box"
pos=$(q "(lambda w: '1400 ' + str(round(w - 192)))(d['window']['h'])")
ri move $pos -- wait 40 -- click -- wait 100
key Control+v; pause 0.3
check "the summary has it" "'with some words' in d['summary']"
shot select-paste
echo "a drag past the text's bottom keeps scrolling while the pointer rests there, and out of the window too"
S0=$(q "d['diff']['scroll']")
ri move 400 860 -- wait 300 -- down -- wait 40 -- move 500 870 -- wait 40 -- move 500 895 -- wait 100
S1=$(q "d['diff']['scroll']"); pause 0.5; S2=$(q "d['diff']['scroll']")
ri up -- wait 100
check_sh "the first move past the edge scrolled ($S0 to $S1)" "python3 -c 'import sys; sys.exit(0 if $S1 > $S0 else 1)'"
check_sh "and it went on at rest ($S1 to $S2)" "python3 -c 'import sys; sys.exit(0 if $S2 > $S1 else 1)'"
check "the selection is many lines" "d['diff']['selected'] and d['diff']['selected'].count(chr(10)) > 10"
S3=$(q "d['diff']['scroll']"); pause 0.2
check "it stopped at the release" "d['diff']['scroll'] == $S3"
finish
