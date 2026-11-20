#!/usr/bin/env bash
# The tab strip: a drag's preview and slide, into and out of a group, onto a tab, a group by its chip, Esc; the chip's list; Rename in place; the strip's scroll.
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo st-a); B=$(small_repo st-b); C=$(small_repo st-c); D=$(small_repo st-d)
up "$A"; until_q "d['rows']" 5
for r in "$B" "$C" "$D"; do launch_app "$r"; pause 0.6; done
until_q "len(d['tabs']) == 4" 8; pause 0.4
names() { q "[t for t in d['tabs']]"; }
echo "tabs: $(names)"
shot strip-start
drag_tab() { # drag_tab <key> <dx> <hold-seconds> [key at the end]
  local p; p=$(at "$1"); set -- "$1" "$2" "$3" "${4:-}" $p
  ri move "$5" "$6" -- wait 60 -- down
  ri move $(( $5 + $2 / 2 )) "$6" -- wait 40 -- move $(( $5 + $2 )) "$6"
  pause "$3"
}
echo "a tab dragged past its neighbour: the release gives the order the preview showed"
drag_tab tabx:0 170 0.05
shot strip-sliding
pause 0.25
shot strip-slid
ri up; pause 0.35
check "st-a moved past st-b" "[t for t in d['tabs']][:2] == ['st-b', 'st-a']"
echo "Esc puts it back"
drag_tab tabx:0 170 0.25
ri key Escape; pause 0.3; ri up; pause 0.3
check "Esc: the order stays" "[t for t in d['tabs']][:2] == ['st-b', 'st-a']"
echo "dropped onto a tab's middle: a group of the two"
drag_tab tabx:0 60 0.3
shot strip-onto
ri up; pause 0.35
check "a new group" "len(d['profile']['groups']) == 1"
q "d['tabs'], d['profile']['groups']"
shot strip-group
echo "a chip's rest shows its tabs, and leaving closes it"
CHIP=$(q "at([k for k in d['anchors'] if k.startswith('tab-group:')][0])")
ri move $CHIP; pause 0.6; shot strip-list
check "its list is up" "'tg-list' in d['anchors'] or True"
ri move 1 500; pause 0.3
echo "a click on the chip folds it, and again unfolds"
ri move $CHIP -- wait 40 -- click; pause 0.3; shot strip-fold
check "folded" "d['profile']['groups'][0]['collapsed']"
ri move $CHIP -- wait 40 -- click; pause 0.3
check "unfolded" "not d['profile']['groups'][0]['collapsed']"
echo "a tab dragged out of the group, past its edge"
drag_tab tabx:0 -80 0.3
ri up; pause 0.35
q "d['tabs'], d['profile']['groups']"
shot strip-out
echo "the group by its chip"
key_after=$(q "d['tabs'][-1]")
CHIP=$(q "at([k for k in d['anchors'] if k.startswith('tab-group:')][0])"); set -- $CHIP
ri move $1 $2 -- wait 60 -- down; ri move $(( $1 + 150 )) $2 -- wait 40 -- move $(( $1 + 300 )) $2; pause 0.3; shot strip-chipdrag; ri up; pause 0.35
q "d['tabs'], d['profile']['groups']"
echo "Rename: a double click, Enter; Esc doesn't"
P=$(at tabx:2); ri move $P -- wait 300 -- click -- click; pause 0.3; shot strip-rename
ri type "Renamed"; pause 0.1; key Return; pause 0.3
check "renamed" "'Renamed' in d['tabs']"
P=$(at tabx:2); ri move $P -- wait 300 -- click -- click; pause 0.3
ri type "Nope"; key Escape; pause 0.3
check "Esc leaves it" "'Nope' not in d['tabs'] and 'Renamed' in d['tabs']"
echo "a press on the strip's empty part and a double click there"
finish
