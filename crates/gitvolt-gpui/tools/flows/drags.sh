#!/usr/bin/env bash
# native: yes
# What a press and a move do: dividers, tabs, chips, split buttons, the rebase editor's rows, the merge tool, Rendered Markdown.
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo drag-a); B=$(small_repo drag-b); C=$(small_repo drag-c)
echo more >>"$A/README.md"
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5; pause 0.3
launch_app "$B"; pause 0.5; launch_app "$C"; until_q "len(d['tabs']) == 3" 5; pause 0.3
key Control+1; pause 0.4
echo "the sidebar's divider"
W0=$(q "d['profile']['sidebarWidth']")
ri move 240 500 -- wait 80; check "the cursor says so" "d['cursor'] == 'ResizeLeftRight'"
ri down -- wait 40 -- move 300 500 -- wait 40 -- move 340 500 -- wait 80 -- up -- wait 200
check "it moved" "d['profile']['sidebarWidth'] == 340"
ri move 340 500 -- wait 300 -- click -- click -- wait 300
check "a double click resets it (the border's pixel aside)" "abs(d['profile']['sidebarWidth'] - $W0) <= 1"
echo "tabs: a drag reorders, Esc takes it back"
T0=$(q "d['tabs']")
ri move 79 47 -- wait 60 -- down -- wait 40 -- move 200 47 -- wait 60 -- move 330 47 -- wait 60; key Escape; pause 0.1; ri up -- wait 200
check "Esc: as it was" "str(d['tabs']) == '''$T0'''"
ri move 79 47 -- wait 60 -- down -- wait 40 -- move 200 47 -- wait 60 -- move 330 47 -- wait 60 -- up -- wait 200
check "moved" "str(d['tabs']) != '''$T0'''"
key Control+3; pause 0.4
echo "chip onto chip, Esc, the menu"
FROM=$(at "chip:1:feature/two"); TO=$(at "chip:0:main")
ri move $FROM -- wait 80 -- down -- wait 50 -- move 320 165 -- wait 50 -- move $TO -- wait 80; key Escape; pause 0.1; ri up -- wait 300
check "Esc: no menu" "d['menu'] == []"
ri move $FROM -- wait 80 -- down -- wait 50 -- move 320 165 -- wait 50 -- move $TO -- wait 80 -- up -- wait 300
check "the drop's menu" "any('Merge feature/two' in r for r in d['menu'][0])"
key Escape; pause 0.2
echo "a double click on a chip checks it out"
ri move $FROM -- wait 200 -- click -- click -- wait 800
check "checked out" "d['head'] == 'feature/two'"
P=$(at "side:refs/heads/main"); ri move $P -- wait 200 -- click -- click -- wait 800
check "and the sidebar's row back" "d['head'] == 'main'"
echo "the split button"
P=$(at tb-sync-caret); ri move $P -- wait 80 -- down -- wait 150; R=$(row_at "Pull (rebase)"); ri move $R -- wait 120 -- up -- wait 300
check "press, drag, release picks" "d['menu'] == []"
P=$(at tb-sync); ri move $P -- wait 80 -- down -- wait 700
check "a hold opens its menu" "len(d['menu']) == 1 and any('Pull (rebase)' in r for r in d['menu'][0])"
ri up -- wait 300; key Escape; pause 0.2
echo "a right press's drag picks"
P=$(rowat "Change 2"); ri move $P -- wait 80 -- down right -- wait 120; R=$(row_at "Create tag"); ri move $R -- wait 100 -- up right -- wait 300
check "the row ran" "d['editor'] is not None"
key Escape; pause 0.2

echo "the rebase editor's rows"
bash "$TOOLS/conflict-fixture.sh" irebase >/dev/null 2>&1; I=$WORK/irebase-fixture
launch_app "$I"; until_q "d['rows'] and d['rows'] >= 6 and d['tabs'][-1] == 'irebase-fixture' or 'irebase-fixture' in d['tabs']" 5; pause 0.8
P=$(at "chip:0:main"); ri move $P -- wait 60 -- click right -- wait 300; R=$(row_at "Interactive rebase"); [ -n "$R" ] || true; key Escape
clickon "side:refs/heads/trunk" right; pause 0.3; pick "Interactive rebase main onto trunk"; pause 0.8
check "the editor's open" "d['rebase'] and d['rebase']['open'] and len(d['rebase']['rows']) == 6"
Y=($(q "' '.join(str(round(b[1] + b[3] / 2)) for k, b in sorted(((k, b) for k, b in d['anchors'].items() if k.startswith('irow:')), key=lambda x: x[1][1]))"))
ROW0=$(q "d['rebase']['rows'][0]")
ri move 700 ${Y[0]} -- wait 80 -- down -- wait 40 -- move 700 $((Y[0] + 20)) -- wait 40 -- move 700 ${Y[3]} -- wait 40 -- move 700 $((Y[3] + 10)) -- wait 100 -- up -- wait 300
check "a drag moved the row" "d['rebase']['rows'][3] == '$ROW0'"
R1=$(q "d['rebase']['rows'][1]")
ri move 700 ${Y[1]} -- wait 80 -- down -- wait 40 -- move 700 $((Y[1] + 20)) -- wait 40 -- move 700 ${Y[3]} -- wait 100; key Escape; pause 0.1; ri up -- wait 300
check "Esc in a drag: the plan stands" "d['rebase']['rows'][1] == '$R1' and d['rebase']['open']"

echo "the merge tool"
bash "$TOOLS/conflict-fixture.sh" merge >/dev/null 2>&1; M=$WORK/merge-fixture
launch_app "$M"; pause 1; until_q "d['operation']" 6
clickxy 700 147; pause 0.4; clickon "tip:parser.rs"; until_q "d['merge'] and d['merge']['path'] == 'parser.rs'" 5; pause 0.5
S0=$(q "d['merge']['scroll'][0]")
ri move 570 400 -- wait 80 -- down -- wait 100 -- up -- wait 100
check "the minimap took the view, both panes" "d['merge']['scroll'][0] != $S0 and d['merge']['scroll'][0] == d['merge']['scroll'][1]"
ri move 613 250 -- wait 80 -- down -- wait 80 -- move 613 300 -- wait 80 -- up -- wait 100
check "the ruler's drag too" "d['merge']['scroll'][0] == d['merge']['scroll'][1]"
key F7; pause 0.3
clickon "$(q "sorted(k for k in d['anchors'] if k.startswith('merge-hunk:0:'))[0]")"; pause 0.3
check "a hunk's tick" "d['merge']['picked'] == 1 and d['merge']['dirty']"
key Control+z; pause 0.3
check "Ctrl+Z takes it back" "d['merge']['picked'] == 0"
ri move 200 300 -- wait 60 -- down -- wait 40 -- move 300 300 -- wait 60 -- move 360 320 -- wait 80 -- up -- wait 100; key Control+c; pause 0.2
check "a pane's selection copies" "d['clipboard'] and len(d['clipboard']) > 3"
ri move 300 340 -- wait 400 -- click -- click -- wait 100; key Control+c; pause 0.2
check "a double click's word, from the pane pressed in" "d['clipboard'] and chr(10) not in d['clipboard']"
ri move 500 720 -- wait 60 -- click -- wait 200; ri type "typed"; pause 0.3
check "typing in the output" "d['merge']['dirty']"
key Control+z; pause 0.3
key Escape; pause 0.3; [ "$(q "d['popover']")" = True ] && { clickon "popover:discard"; pause 0.2; [ "$(q "d['popover']")" = True ] && clickon "popover:discard"; }; pause 0.3

echo "Rendered Markdown"
K=$WORK/md-fixture
launch_app "$K"; pause 1; until_q "'md-fixture' in d['tabs'] and d['rows'] == 3" 6
clickrow "New versions"; pause 0.5; clickon "tip:big.md"; until_q "d['md'] and d['md']['ready']" 6; pause 0.5
ri move 300 400 -- wait 100 -- down -- wait 50 -- move 400 400 -- wait 60 -- move 500 440 -- wait 100 -- up -- wait 150
check "a drag selects" "d['md']['selected']"
ri move 400 300 -- wait 500 -- click -- click -- wait 150
check "a double click, a word" "d['md']['selected'] and ' ' not in d['md']['selected'].strip()"
key Control+a; pause 0.2; check "Ctrl+A" "len(d['md']['selected'] or '') > 1000"
ri move 500 800 -- wait 400 -- click -- wait 50 -- down -- wait 40 -- move 520 860 -- wait 40 -- move 520 895 -- wait 100
A0=$(q "d['md']['scroll']"); pause 0.5; A1=$(q "d['md']['scroll']"); ri up -- wait 100
check_sh "a drag past the pane keeps coming ($A0 to $A1)" "python3 -c 'import sys; sys.exit(0 if $A1 > $A0 else 1)'"
finish
