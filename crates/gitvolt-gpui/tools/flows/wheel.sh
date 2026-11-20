#!/usr/bin/env bash
# Wayland: the fake device's wheel has no notch (no `value120`), and GPUI drops that, so a wheel on Wayland can't run here.
# The wheel over each scrolling surface: the same step, the right way round, nothing at the ends.
source "$(dirname "$0")/lib.sh"
clean_app
L=$(long_repo wheel 600)
up "$L"; until_q "d['rows'] and d['rows'] > 100" 10
step() { # A notch's difference after a first notch that may only teach an X11 client its baseline.
  local expr=$1 a b
  ri wheel down 1 -- wait 120
  a=$(q "$expr"); ri wheel down 1 -- wait 150; b=$(q "$expr")
  python3 -c "print(round(abs(float('$b') - float('$a')), 1))"
}
echo "graph"
ri move 700 400 -- wait 80
s=$(step "d['listOffset']"); check_sh "a notch of the graph is 53 px ($s)" "[ ${s%.*} = 53 ]"
check "down scrolls down" "d['listOffset'] < 0"
ri wheel up 40 -- wait 200
check "the top stops it" "d['listOffset'] == 0"
echo "sidebar"
ri move 100 200 -- wait 80
ri wheel down 2 -- wait 150
shot wheel-side
echo "diff"
clickxy 700 147; until_q "d['wip'] and d['wip']['unstaged']" 5
clickon "tip:long.txt"; until_q "d['open'] == 'long.txt' and d['diff']" 5; pause 0.5
ri move 700 500 -- wait 80
s=$(step "d['diff']['scroll']"); check_sh "a notch of a diff is Monaco's 50 px ($s)" "[ ${s%.*} = 50 ]"
A=$(q "d['diff']['scroll']"); ri wheel up 1 -- wait 150
check "up comes back" "d['diff']['scroll'] < $A"
shot wheel-diff
echo "the file list while a diff shows"
ri move 1400 300 -- wait 80; ri wheel down 2 -- wait 150
check "the diff doesn't move for the list's wheel" "d['diff']['scroll'] < $A"
echo "a menu"
clickon "tb-undo-caret"; pause 0.2
ri wheel down 2 -- wait 200
check "the wheel leaves the app running with a menu" "True"
key Escape
finish
