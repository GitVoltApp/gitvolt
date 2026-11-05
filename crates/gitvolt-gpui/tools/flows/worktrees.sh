#!/usr/bin/env bash
# Worktrees, checked out elsewhere, tab groups, profiles, and the Open screen's banner.
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo wt); W=$WORK/flows/wt-other; rm -rf "$W"
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5; pause 0.3
echo "Create worktree from a branch's menu"
clickon "side:refs/heads/feature/one" right; pause 0.25; pick "Create worktree from"; pause 0.5
pick "feature/one"; pause 0.6
q "d['modals']"; shot wt-create
ri key Control+a; ri type "$W"; pause 0.2
key Return; pause 1.5
q "d['toast'], d['modals'], d['tabs']"
check_sh "the worktree exists" "git -C '$W' rev-parse --abbrev-ref HEAD | grep -q feature/one"
echo "a double click on what's checked out elsewhere goes to that worktree, in the tab"
P=$(at "side:refs/heads/feature/one"); ri move $P -- wait 300 -- click -- click -- wait 900
check "the tab's on the other worktree" "any('wt-other' in w for w in d['profile']['worktrees']) and d['head'] == 'feature/one'"
Q=$(q "at([k for k in d['anchors'] if k.startswith('side:wt:') and not k.endswith('wt-other')][0])"); ri move $Q -- wait 300 -- click -- click -- wait 900
check "and back by the worktree's row" "not any('wt-other' in w for w in d['profile']['worktrees']) and d['head'] == 'main'"
echo "Checkout by the menu: the toast's links"
clickon "chip:2:feature/one" right; pause 0.3; q "[r['label'] for r in d['menuRows'] if 'heckout' in r['label']]"; pick "Checkout"; pause 0.5; q "d['menu'][-1]"; shot wt-checkout
pick "feature/one"; pause 1
q "d['toast'], d['head']"; shot wt-elsewhere
ACTS=$(q "[k for k in d['anchors'] if k.startswith('toast-action')]"); echo "$ACTS"
L=$(q "at([k for k in d['anchors'] if k.startswith('toast-action') and 'new tab' in k][0])"); echo "link at $L"
ri move $L -- wait 60 -- click -- wait 1000
check "Open in a new tab: two tabs, the new one on that worktree" "len(d['tabs']) == 2 and 'wt-other' in d['profile']['worktrees'][-1]"
key Control+w; pause 0.6
check "and it closes" "len(d['tabs']) == 1"
echo "Remove worktree With branch"
Q=$(q "at([k for k in d['anchors'] if k.startswith('side:wt:') and 'wt-other' in k][0])")
ri move $Q -- wait 60 -- click right -- wait 300; q "d['menu'][0]"; q "[r for r in d['menuRows'] if 'Remove' in r['label']]"; shot wt-remove
pickv "Remove" "With branch"; pause 0.4; q "d['armed']"; shot wt-remove2
pickv "Remove" "With branch"; pause 1.5
check_sh "the worktree's gone" "[ ! -d '$W' ]"
check_sh "and its branch" "! git -C '$A' rev-parse -q --verify refs/heads/feature/one"

echo "Tab groups: saved and brought back"
B=$(small_repo wt-b); launch_app "$B"; until_q "len(d['tabs']) == 2" 5; pause 0.3
clickon "tabx:0" right; pause 0.3; pick "Add to new group"; pause 0.6
check "a group of one" "len(d['profile']['groups']) == 1"
shot wt-group
CHIP=$(q "at([k for k in d['anchors'] if k.startswith('tab-group:')][0])")
ri move $CHIP -- wait 60 -- click right -- wait 400; q "sorted(k for k in d['anchors'] if k.startswith('tg-'))"; shot wt-groupmenu
clickon tg-save; pause 0.7
check "Save and close: the tab's gone, the group saved" "d['profile']['saved'] == 1 and len(d['tabs']) == 1"
echo "brought back from the tab bar's menu"
clickxy 800 47 right; pause 0.3; q "d['menu'][0]"; shot wt-barmenu
P=$(row_at "Saved groups"); ri move $P -- wait 400; q "d['menu']"; shot wt-saved
SUB=$(q "d['menuRows'][-1]['label']"); echo "saved row: $SUB"
pick "$SUB"; pause 0.4; q "d['menu'][-1] if d['menu'] else None"; shot wt-saved2
[ "$(q "bool(d['menu'])")" = True ] && { pick "Open"; pause 1; }
q "d['tabs'], d['profile']['saved'], d['profile']['groups']"
check "the group's tab came back" "len(d['tabs']) == 2 and len(d['profile']['groups']) == 1"
shot wt-group-back

echo "profiles: new, switch, delete"
clickon profile-switcher; pause 0.3; q "d['menu'][0]"; shot wt-profiles
pick "New profile"; pause 0.5; q "d['modals']"
ri type "Second"; pause 0.2; key Return; pause 1
q "d['profile']['name'], d['profile']['count'], d['tabs']"
check "on the new profile" "d['profile']['name'] == 'Second' and d['profile']['count'] == 2"
clickon profile-switcher; pause 0.3
P=$(row_at "Delete profile"); ri move $P -- wait 400
pick "Default"; pause 0.3; q "d['armed']"; shot wt-profile-del
pick "Default"; pause 1
check "the other's deleted" "d['profile']['count'] == 1 and d['profile']['name'] == 'Second'"
finish
