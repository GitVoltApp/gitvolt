#!/usr/bin/env bash
# native: yes
# Branches, tags, remotes and resets by their menus, with a real mouse; checked in the fixture's git state.
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo refs); O=$WORK/flows/refs-origin.git; rm -rf "$O"; git init -q --bare "$O"
g "$A" remote add origin "$O"; g "$A" config protocol.file.allow always; g "$A" push -q origin main feature/one
g "$A" branch --set-upstream-to origin/feature/one feature/one >/dev/null
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5; pause 0.5
head() { g "$A" rev-parse --short "$1"; }
rmenu() { P=$(rowat "$1"); ri move $P -- wait 60 -- click right -- wait 250; }
choose() { pick "$1"; pause "${2:-0.6}"; }
sidemenu() { clickon "$1" right; pause 0.25; }

echo "fast-forward: feature/two behind main"
sidemenu "side:refs/heads/feature/two"
q "d['menu'][0][:12]"
choose "Fast-forward feature/two"
check_sh "feature/two moved to main" "[ $(head main) = \$(git -C '$A' rev-parse --short feature/two) ]"
g "$A" branch -f feature/two HEAD~1; pause 0.7

echo "chip drag's menu row picked: merge"
g "$A" checkout -q -b drag-base HEAD~2; echo x >"$A/d.txt"; g "$A" add d.txt; g "$A" commit -qm "Drag base"; g "$A" checkout -q main; pause 0.8
FROM=$(at "chip:0:drag-base"); TO=$(q "at([k for k in d['anchors'] if k.endswith(':main')][0])")
ri move $FROM -- wait 60 -- down -- wait 40 -- move 320 300 -- wait 40 -- move $TO -- wait 60 -- up -- wait 300
q "d['menu']"
choose "Merge drag-base into main" 1.2
q "d['armed'], d['toast'], d['popover']"
check_sh "the merge happened" "git -C '$A' log -1 --format=%s | grep -q 'Merge branch'"
key Control+z; pause 1.2
check_sh "and Undo took it back" "! git -C '$A' log -1 --format=%s | grep -q 'Merge branch'"

echo "soft reset"
TIP=$(head main)
rmenu "Change 4"; q "[r for r in d['menu'][0] if 'Reset' in r or 'reset' in r]"
q "[r for r in d['menuRows'] if 'Reset' in r['label']][0]['variants']"
pickv "Reset main to this commit" Soft; pause 1
check_sh "soft: the commit's gone, its change staged" "git -C '$A' log -1 --format=%s | grep -q 'Change 4' && git -C '$A' diff --cached --quiet; [ \$? = 1 ]"
key Control+z; pause 1
check_sh "undone" "[ $TIP = \$(git -C '$A' rev-parse --short main) ]"

echo "mixed reset"
rmenu "Change 4"; pickv "Reset main to this commit" Mixed; pause 1
check_sh "mixed: the commit's gone, its change unstaged" "git -C '$A' log -1 --format=%s | grep -q 'Change 4' && git -C '$A' diff --quiet; [ \$? = 1 ]"
key Control+z; pause 1

echo "Set upstream on a branch"
g "$A" push -q origin feature/two; pause 0.8
sidemenu "side:refs/heads/feature/two"; choose "Set upstream" 0.5
q "d['picker'][:4]"
shot refs-upstream
choose "origin/feature/two" 1
check_sh "tracking" "git -C '$A' rev-parse --abbrev-ref feature/two@{upstream} | grep -q origin/feature/two"

echo "tag: push, delete on the remote, delete"
g "$A" tag v3.0.0 main; pause 0.8
sidemenu "side:refs/tags/v3.0.0"; q "d['menu'][0]"
choose "Push" 1.2
check_sh "the tag is on the remote" "git -C '$O' rev-parse -q --verify refs/tags/v3.0.0"
sidemenu "side:refs/tags/v3.0.0"; q "[r for r in d['menuRows'] if 'Delete' in r['label']][0]['variants']"; shot refs-tag-del
pickv Delete Remote; pause 0.3; q "d['armed']"; pickv Delete Remote; pause 1.2
check_sh "gone from the remote, kept here" "! git -C '$O' rev-parse -q --verify refs/tags/v3.0.0 && git -C '$A' rev-parse -q --verify refs/tags/v3.0.0"
sidemenu "side:refs/tags/v3.0.0"; pick Delete; pause 1
check_sh "gone here" "! git -C '$A' rev-parse -q --verify refs/tags/v3.0.0"

echo "the remote: Copy URL, Remove confirmed"
q "sorted(k for k in d['anchors'] if k.startswith('side:'))[:12]"
# The remote's own row has no anchor: five rows above its last branch.
R=$(q "(lambda a: str(round(a[0])) + ' ' + str(round(a[1] - 24 * 4)))([float(v) for v in at('side:refs/remotes/origin/main').split()])")
echo "origin at $R"
ri move $R -- wait 60 -- click right -- wait 250; q "d['menu'][0]"; shot refs-remote
pick "Copy URL"; pause 0.3
check "Copy URL" "d['clipboard'] and 'refs-origin.git' in d['clipboard']"
ri move $R -- wait 60 -- click right -- wait 250; pick "Remove"; pause 0.3; q "d['armed'],d['popover']"; shot refs-remove
pick "Remove"; pause 1
check_sh "the remote's removed" "! git -C '$A' remote | grep -q origin"

echo "compare, and its swap"
clickrow "Change 5"; pause 0.3
P=$(rowat "Change 2"); ri hold Control -- wait 30 -- move $P -- wait 50 -- click -- wait 30 -- release Control; pause 0.5
check "two picked: a compare" "d['compare']"
q "d['compare']"
shot refs-compare
B=$(q "d['compare'][:30]")
clickon compare-swap; pause 0.5
check "swapped" "d['compare'] and d['compare'][:30] != '$B'"
key Escape; pause 0.3
check "Esc leaves the compare" "not d['compare']"
finish
