#!/usr/bin/env bash
# native: yes
# A diff's overview ruler held and dragged off the strip, and a maximized window's edges.
source "$(dirname "$0")/lib.sh"
clean_app
L=$(long_repo ruler 40)
up "$L"; until_q "d['rows'] and d['rows'] > 10" 10
clickxy 700 147; until_q "d['wip'] and d['wip']['unstaged']" 5
clickon "tip:long.txt"; until_q "d['open'] == 'long.txt' and d['diff']" 5; pause 0.5
ruler() { q "(lambda b: '%d %d %d %d' % (b[0], b[1], b[2], b[3]))(d['anchors']['diff-ruler'])"; }
read -r RX RY RW RH <<<"$(ruler)"
echo "ruler at $RX $RY ${RW}x$RH"
MID=$((RX + RW / 2))
ri move "$MID" $((RY + RH / 2)) -- wait 60 -- click; pause 0.2
BEFORE=$(q "d['diff']['scroll']")
echo "a press on the strip centres there: $BEFORE"
check "a press off the slider moved the view" "d['diff']['scroll'] > d['diff']['max'] * 0.4 and d['diff']['scroll'] < d['diff']['max'] * 0.6"
shot ruler-pressed
echo "holding the slider doesn't move the view"
ri move "$MID" $((RY + RH / 2 + 2)) -- wait 60 -- down; pause 0.2
check "held" "d['diff']['drag'] == 'Ruler'"
check "still where it was" "abs(d['diff']['scroll'] - $BEFORE) < 200"
echo "off the strip, to the left and down"
ri move $((RX - 400)) $((RY + RH * 3 / 4)) -- wait 120
check "it followed off the strip" "d['diff']['scroll'] > $BEFORE + d['diff']['max'] * 0.1"
check "still held" "d['diff']['drag'] == 'Ruler'"
shot ruler-off
ri move $((RX - 600)) $((RY + 5)) -- wait 120
check "and back up" "d['diff']['scroll'] < d['diff']['max'] * 0.1"
ri up; pause 0.2
check "let go" "d['diff']['drag'] == 'None'"
AFTER=$(q "d['diff']['scroll']")
ri move 700 400 -- wait 120
check "a move after it does nothing" "abs(d['diff']['scroll'] - $AFTER) < 1"
echo "the open file's row closes it, and opens it again"
clickon "tip:long.txt"; until_q "d['diff'] is None" 3
check "closed, by its row" "d['diff'] is None"
clickon "tip:long.txt"; until_q "d['diff'] is not None" 5; pause 0.4
check "open again" "d['open'] == 'long.txt'"
echo "File View's minimap: its slider stays under the pointer"
ri move 590 161 -- wait 80 -- click -- wait 900
check "File View" "d['diff']['view'] == 'File'"
ri move 700 400 -- wait 100; ri wheel down 20 -- wait 300
MAPX=1140
ri move "$MAPX" 300 -- wait 200; shot ruler-map0
BEFORE=$(q "d['diff']['scroll']")
ri move "$MAPX" 200 -- wait 120 -- down -- wait 120
shot ruler-map1
START=$(q "d['diff']['scroll']")
check "held on the map" "d['diff']['drag'] == 'Ruler'"
ri move $((MAPX - 500)) 500 -- wait 150
check "followed off the map" "d['diff']['scroll'] > $START + 500"
shot ruler-map2
ri up -- wait 150
check "let go on the map" "d['diff']['drag'] == 'None'"
echo "the graph's scrollbar, off its strip"
clickon "tip:long.txt"; until_q "d['diff'] is None" 3; pause 0.3
G=$(q "(lambda b: '%d %d %d' % (b[0], b[1], b[3]))(d['anchors'].get('graph-scrollbar'))")
read -r GX GY GH <<<"$G"
Y0=$(q "d['graphScroll']")
ri move $((GX + 4)) $((GY + GH / 4)) -- wait 100 -- down -- wait 100
Y1=$(q "d['graphScroll']")
check "held on the thumb, the graph stays" "abs(d['graphScroll'] - $Y0) < 1"
ri move $((GX - 500)) $((GY + GH * 3 / 4)) -- wait 150
check "it followed off the strip" "d['graphScroll'] > $Y1 + 200"
ri move $((GX - 500)) $((GY + 2)) -- wait 150
check "and back" "d['graphScroll'] < 500"
ri up -- wait 150
Y2=$(q "d['graphScroll']")
ri move 700 500 -- wait 150
check "a move after it does nothing" "abs(d['graphScroll'] - $Y2) < 1"
echo "maximized: no edge resizes"
clickon win-maximize; until_q "d['window']['maximized']" 5; pause 0.5
W=$(q "int(d['window']['w'])"); H=$(q "int(d['window']['h'])")
ri move 1 $((H / 2)) -- wait 100 -- down -- wait 60 -- move 200 $((H / 2)) -- wait 100 -- up -- wait 300
check "the left edge is no handle" "d['window']['maximized'] and int(d['window']['w']) == $W"
ri move $((W / 2)) $((H - 1)) -- wait 100 -- down -- wait 60 -- move $((W / 2)) $((H - 200)) -- wait 100 -- up -- wait 300
check "nor the bottom" "d['window']['maximized'] and int(d['window']['h']) == $H"
shot ruler-max
key Escape
clickon win-maximize; until_q "not d['window']['maximized']" 5
finish
