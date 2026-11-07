#!/usr/bin/env bash
# Rows the sweep hadn't run: the Open screen banner's Use, the action queue's Resume and Clear, and on the fake forge auto-merge, a saved draft's
# Discard and forks' load-more. What a real input can't reach here (a folder dialog's answer) or that wants more of a fixture (LFS storage, stacks that merge) isn't in it.
source "$(dirname "$0")/lib.sh"
clean_app
echo "the Open screen's banner: Use"
mkdir -p "$REALINPUT_ROOT/app/home/repos"
up; until_q "d['openScreen'] is not None" 5; pause 0.5
check "Use is offered" "'open-use' in d['anchors']"
clickon open-use; pause 0.8
check "Use takes it: the banner's gone" "'open-use' not in d['anchors'] and 'open-choose' not in d['anchors']"
check_sh "and it's the profile's" "grep -q 'reposFolder.*repos' '$REALINPUT_ROOT/app/config/gitvolt-gpui/profiles/default/profile.json' || { sleep 1; grep -q 'reposFolder.*repos' '$REALINPUT_ROOT/app/config/gitvolt-gpui/profiles/default/profile.json'; }"
shot left-use
stop_app

echo "the queue: what a failure leaves, Clear and Resume"
A=$(small_repo queue)
printf '#!/bin/sh\nsleep 1.5\nexit 1\n' >"$WORK/flows/fail-ssh.sh"; chmod +x "$WORK/flows/fail-ssh.sh"
g "$A" remote add origin ssh://git@forge.example.com/queue.git; g "$A" config core.sshCommand "$WORK/flows/fail-ssh.sh"
g "$A" update-ref refs/remotes/origin/main HEAD~1; g "$A" branch -q -u origin/main main
clean_app
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5; pause 0.5
echo "Alt+click on a folder hides what's under it, and doesn't fold it"
check "its branches' chips are there" "any('feature/one' in k for k in d['anchors'] if k.startswith('chip:'))"
ri hold Alt -- wait 30 -- move 48 186 -- wait 60 -- click -- wait 30 -- release Alt; pause 0.8
shot left-folder-hidden
check "they've gone from the graph" "not any('feature/one' in k for k in d['anchors'] if k.startswith('chip:'))"
check "the folder's still open" "'side:refs/heads/feature/one' in d['anchors']"
ri hold Alt -- wait 30 -- move 48 186 -- wait 60 -- click -- wait 30 -- release Alt; pause 0.8
check "and again shows them" "any('feature/one' in k for k in d['anchors'] if k.startswith('chip:'))"
stopped_queue() {
  clickon tb-sync; pause 0.2; clickon tb-push; pause 0.3
  until_q "'queue-chip' in d['anchors'] and not d['running']" 8
}
stopped_queue
shot left-queue
clickon queue-chip; pause 0.3
q "[r['label'] for r in d['menuRows']]"
check "Resume and Clear are offered" "any(r['label'] == 'Resume' for r in d['menuRows']) and any(r['label'] == 'Clear' for r in d['menuRows'])"
pick Clear; pause 0.6
check "Clear: the queue's gone" "'queue-chip' not in d['anchors']"
stopped_queue
clickon queue-chip; pause 0.3
pick Resume; pause 0.3
until_q "not d['running']" 8
check "Resume ran what was left" "len(d['toast'] or '') >= 0"
shot left-resume
stop_app

echo "the update dialog's Restart asks about unsaved edits, and the dialog stays if that's cancelled"
U=$(small_repo upd)
clean_app
export GITVOLT_FAKE_UPDATE=installed
up "$U"; until_q "d['rows'] and d['rows'] >= 6" 5; pause 0.3
echo "work" >>"$U/src/main.txt"; until_q "d['rows'] and d['rows'] >= 7" 5
clickxy 700 147; until_q "d['wip'] and d['wip']['unstaged']" 5
clickon "tip:src/main.txt" || { clickon "tip:src"; pause 0.3; clickon "tip:src/main.txt"; }
until_q "d['open'] == 'src/main.txt'" 5; pause 0.3
clickxy 800 500; pause 0.2; key End; ri type "UNSAVED"; pause 0.3
clickon status-update; pause 0.4
check "the dialog offers Restart" "'update-restart' in d['anchors']"
clickon update-restart; pause 0.4
check "it asks" "d['popover'] and 'popover:discard' in d['anchors']"
shot left-update-ask
key Escape; pause 0.3
check "cancelled: the dialog's still up, the same process" "not d['popover'] and 'update-restart' in d['anchors'] and d['pid'] == $APP_PID"
clickon update-restart; pause 0.4; clickon popover:discard; pause 0.3; [ "$(q "d['popover']")" = True ] && clickon popover:discard; pause 0.6
check "Discard goes on to the core's request, which has no update here" "'installed' in (d['toast'] or '')"
stop_app
unset GITVOLT_FAKE_UPDATE

export GITVOLT_E2E_PORT_BASE=${GITVOLT_E2E_PORT_BASE:-7446}
LAB="$TOOLS/forge-lab.sh"
"$LAB" start >/dev/null
trap '"$LAB" stop' EXIT
H=http://127.0.0.1:$GITVOLT_E2E_PORT_BASE
curl -sf -X POST "$H/test/reset" >/dev/null
export GITVOLT_TEST_FORGE=$("$LAB" base)
# Twelve more forks for a second page, a running pipeline on MR 12.
curl -sf "$H/test/forge/seed" | python3 -c "
import json, sys, copy
s = json.load(sys.stdin)
g = s['gitlab']
fork = next(p for p in g['projects'] if p['path'] == 'alice/project')
for i in range(12):
    f = copy.deepcopy(fork); f['id'] = 200 + i; f['path'] = f'fork{i}/project'; g['projects'].append(f)
mr = next(m for m in g['mergeRequests'] if m['iid'] == 12)
mr['pipeline'] = 'running'
print(json.dumps(s))" | curl -sf -X POST -H 'content-type: application/json' -d @- "$H/test/forge/seed" >/dev/null
G=$("$LAB" fixture flow-left gitlab.example.com group/project | tail -1)
clean_app
up "$G"; until_q "d['rows'] and d['rows'] >= 2" 8; pause 0.5
key Control+comma; pause 0.4; clickon settings-nav:accounts; pause 0.5
clickon account-add-first; pause 0.3; ri type "glpat-FAKE-e2e-ada"; pause 0.2; clickon account-submit; pause 1.2
key Escape; pause 0.5
until_q "d['forge']['me'] == 'ada'" 8

echo "forks: load more"
clickon "side:refs/remotes/origin/main" right 2>/dev/null; pause 0.1; key Escape; pause 0.1
key Control+p; pause 0.3; ri type ">Add remote"; pause 0.4; key Return; pause 1.5
shot left-forks
q "sorted(k for k in d['anchors'] if k.startswith('fork-'))[:14]"
check "the first page" "len([k for k in d['anchors'] if k.startswith('fork-add:')]) >= 10"
for i in 1 2 3 4 5 6; do ri move 800 600 -- wait 40; ri wheel down 6 -- wait 200; done; pause 1
q "len([k for k in d['anchors'] if k.startswith('fork-add:')])"
shot left-forks2
check "the second page came as the end neared view" "len([k for k in d['anchors'] if k.startswith('fork-add:')]) > 10"
key Escape; pause 0.4

echo "Create's saved draft: Discard"
clickon "side:refs/heads/feature/new" right; pause 0.3
P=$(row_at "merge request"); [ -n "$P" ] && ri move $P -- wait 40 -- click; pause 1.2
Q=$(q "at('create-title')"); [ -n "$Q" ] && { ri move $Q -- wait 40 -- click; pause 0.2; ri type " draft text"; pause 0.8; }
shot left-draft
key Escape; pause 0.6
clickon "side:refs/heads/feature/new" right; pause 0.3
P=$(row_at "merge request"); [ -n "$P" ] && ri move $P -- wait 40 -- click; pause 1.2
q "sorted(k for k in d['anchors'] if k.startswith('create-'))[:20]"
check "the draft's kept: Discard draft is there" "'create-discard' in d['anchors']"
clickon create-discard; pause 0.3; clickon create-discard; pause 0.6
shot left-draft-discard
check "Discard takes it" "'create-discard' not in d['anchors']"
key Escape; pause 0.4

echo "auto-merge"
clickon "side:mr:12"; pause 1.5
clickon mr-approve; pause 1
shot left-mr
clickon mr-merge-act; pause 0.4; shot left-auto-armed
[ "$(q "bool(d['armed'])")" = True ] && clickon mr-merge-act; pause 1.2
shot left-auto-set
check_sh "the forge was asked to merge when it passes" "'$LAB' requests | grep -q 'merge_requests/12/merge'"
check_sh "and it's set there" "'$LAB' seed | python3 -c \"import json,sys; s=json.load(sys.stdin); sys.exit(0 if next(m for m in s['gitlab']['mergeRequests'] if m['iid']==12).get('autoMergeBy') else 1)\""
clickon mr-merge-act; pause 0.4; [ "$(q "bool(d['armed'])")" = True ] && clickon mr-merge-act; pause 1.2
shot left-auto-cancel
check_sh "Cancel takes it back" "'$LAB' seed | python3 -c \"import json,sys; s=json.load(sys.stdin); sys.exit(1 if next(m for m in s['gitlab']['mergeRequests'] if m['iid']==12).get('autoMergeBy') else 0)\""
stop_app
finish
