#!/usr/bin/env bash
# native: yes
# Stops by real input: a cherry-pick's and a revert's conflict through the merge tool; hiding with Alt+click and the hidden list.
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo ops)
g "$A" checkout -q -b topic HEAD~2; sed -i 's/^line 20 of/TOPIC 20 of/' "$A/src/main.txt"; g "$A" add src; g "$A" commit -qm "Topic edits line 20"; g "$A" checkout -q main
sed -i 's/^line 20 of/MAIN 20 of/' "$A/src/main.txt"; g "$A" add src; g "$A" commit -qm "Main edits line 20"
up "$A"; until_q "d['rows'] and d['rows'] >= 7" 5; pause 0.4
resolve() { # opens the conflicted file, takes the hunk from Current, saves
  until_q "d['wip'] and d['wip']['conflicted']" 5
  clickon "tip:src/main.txt" || { clickon "tip:src"; pause 0.2; clickon "tip:src/main.txt"; }
  until_q "d['merge'] and d['merge']['path'] == 'src/main.txt'" 5; pause 0.5
  clickon "merge-hunk:1:1"; pause 0.3
  check "$1: ticked" "d['merge']['picked'] == 1"
  shot "ops-$1"
  clickon merge-save; pause 0.9
  until_q "d['wip'] and not d['wip']['conflicted']" 5
}
echo "cherry-pick stop"
P=$(rowat "Topic edits line 20"); ri move $P -- wait 80 -- click right -- wait 250
pick "Cherry-pick onto main"; pause 1.2
q "d['operation'], d['toast'], d['wip']"
check "stopped" "d['operation'] and 'herry' in str(d['operation'])"
resolve cherry
q "d['operation'], d['merge'] and d['merge']['path']"
shot ops-cherry-after
cont() { clickxy 1400 "$(q "round(d['window']['h'] - 80)")"; pause 1.5; }
cont
q "d['operation'], d['toast'], d['armed']"
[ "$(q "d['armed']")" = True ] && cont
check_sh "the pick landed" "git -C '$A' log -1 --format=%s | grep -q 'Topic edits line 20' && ! git -C '$A' rev-parse -q --verify CHERRY_PICK_HEAD"
echo "revert stop"
sed -i 's/^TOPIC 20 of/AGAIN 20 of/' "$A/src/main.txt"; g "$A" add src; g "$A" commit -qm "Main edits line 20 again"; pause 1
P=$(rowat "Topic edits line 20"); ri move $P -- wait 80 -- click right -- wait 250
pick "Revert this commit"; pause 1.2
q "d['operation'], d['toast']"
check "stopped" "d['operation']"
resolve revert
cont
[ "$(q "d['armed']")" = True ] && cont
check_sh "the revert landed" "git -C '$A' log -1 --format=%s | grep -q 'Revert' && ! git -C '$A' rev-parse -q --verify REVERT_HEAD"
echo "Alt+click hides a branch; the hidden list; Show"
clickon "tip:side:refs/heads/feature/two" 2>/dev/null; pause 0.1
P=$(at "side:refs/heads/feature/two"); ri hold Alt -- wait 30 -- move $P -- wait 60 -- click -- wait 30 -- release Alt; pause 0.8
q "d['toast'], sorted(k for k in d['anchors'] if 'chip' in k)[:5]"
check "its chip left the graph" "not any(k.endswith(':feature/two') and k.startswith('chip') for k in d['anchors'])"
shot ops-hidden
clickon preset-caret; pause 0.3; q "d['menu'][0][:8]"; shot ops-preset; key Escape
finish
