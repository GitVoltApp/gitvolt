#!/usr/bin/env bash
# The forge's rows by real input, against the harness's fake forge on 127.0.0.1 only, checked in its request log.
source "$(dirname "$0")/lib.sh"
export GITVOLT_E2E_PORT_BASE=${GITVOLT_E2E_PORT_BASE:-7443}
LAB="$TOOLS/forge-lab.sh"
"$LAB" start >/dev/null
trap '"$LAB" stop' EXIT
curl -sf -X POST "http://127.0.0.1:$GITVOLT_E2E_PORT_BASE/test/reset" >/dev/null
export GITVOLT_TEST_FORGE=$("$LAB" base)
G=$("$LAB" fixture flow-gl gitlab.example.com group/project | tail -1)
clean_app
up "$G"; until_q "d['rows'] and d['rows'] >= 2" 8; pause 1
reqs() { "$LAB" requests; }
asked() { reqs | python3 -c "
import json,sys,re
r=json.load(sys.stdin)
r=r if isinstance(r,list) else r.get('requests',r)
sys.exit(0 if any(re.search(r'''$1''', (x.get('method','')+' '+x.get('path',x.get('url','')))) for x in r) else 1)"; }
checkreq() { CHECKS=$((CHECKS + 1)); if asked "$2"; then pass "$1"; else fail "$1 [no request /$2/]"; fi; }
echo "an account by the settings page"
key Control+comma; pause 0.4; clickon settings-nav:accounts; pause 0.5
clickon account-add-first; pause 0.3; ri type "glpat-FAKE-e2e-ada"; pause 0.2; clickon account-submit; pause 1.2; shot forge-account
key Escape; pause 2
until_q "d['forge']['me'] == 'ada'" 6
mr() { [ "$(q "d['forge']['openMr']")" = 12 ] || { clickon "side:mr:12"; pause 1.5; }; ri move 500 500 -- wait 60; ri wheel up 30 -- wait 200; }
visible() { # wheels the view until an anchor's in the window
  local i
  for i in 1 2 3 4 5 6 7 8 9 10 11 12; do
    [ "$(q "(lambda b: b is not None and b[1] > 120 and b[1] + b[3] < d['window']['h'] - 30)(d['anchors'].get('$1'))")" = True ] && return 0
    ri wheel down 3 -- wait 120
  done
  fail "never came into view: $1"; return 1
}
echo "the MR view: open, wheel"
mr
check "it opened" "d['forge']['openMr'] == 12"
ri move 500 600 -- wait 80; ri wheel down 1 -- wait 60; ri wheel down 4 -- wait 200; shot forge-scrolled
echo "approve, unapprove"
mr; clickon mr-approve; pause 1; shot forge-approve
q "d['toast'], d['armed']"
check_sh "approved (the log's last write)" "'$LAB' requests | grep -q -E 'approve|graphql'"
echo "people: add a reviewer"
mr; clickon mr-add-reviewers; pause 0.6; shot forge-people
ri type "gra"; pause 0.6; shot forge-people2; key Return; pause 1
checkreq "the reviewers changed" "PUT .*merge_requests/12$|POST .*reviewers|POST .*graphql"
mr
echo "emoji"
mr; visible "mr-note:101:react" && { clickon "mr-note:101:react"; pause 0.5; shot forge-emoji; ri type "thu"; pause 0.8; shot forge-emoji2; key Return; pause 1; }
q "d['toast']"
check_sh "a reaction (GraphQL)" "'$LAB' requests | grep -q graphql"
echo "a note"
mr; visible "mr-field:new" && { clickon "mr-field:new"; pause 0.2; ri type "A note by a keyboard."; pause 0.3; clickon "mr-reply-send:new"; pause 1.2; }
check_sh "the note went (GraphQL)" "'$LAB' requests | grep -q 'graphql'"
echo "reply and resolve"
mr; visible "mr-reply-open:d2" && { clickon "mr-reply-open:d2"; pause 0.4; ri type "A reply."; pause 0.2; shot forge-reply; visible "mr-reply-send:d2" && clickon "mr-reply-send:d2"; pause 1; }
checkreq "the reply went" "POST .*discussions/d2/notes"
mr; visible "mr-resolve:d2" && { clickon "mr-resolve:d2"; pause 1; }
checkreq "resolved" "PUT .*discussions/d2"
echo "my own note: edit, delete"
mr; N=$(q "sorted((k for k in d['anchors'] if k.endswith(':more') and k.startswith('mr-note:')), key=lambda k: int(k.split(':')[1]))[-1]"); echo "last note: $N"; visible "$N" && { clickon "$N"; pause 0.3; q "d['menu'][0]"; shot forge-note-menu; }
P=$(row_at "Edit"); if [ -n "$P" ]; then ri move $P -- wait 40 -- click; pause 0.4; shot forge-note-edit; key End; ri type " edited"; pause 0.2; key Control+Return; pause 1
  checkreq "the edit went" "PUT .*notes/"
  mr; visible "$N" && { clickon "$N"; pause 0.3; P=$(row_at "Delete"); ri move $P -- wait 40 -- click; pause 0.3; q "d['armed']"; P=$(row_at "Delete"); [ -n "$P" ] && ri move $P -- wait 40 -- click; pause 1; }
  checkreq "the delete went" "DELETE .*notes/"
else
  key Escape; pause 0.2; fail "that note has no Edit"
fi
echo "close the view; the narrow strip"
mr; clickon mr-close; pause 0.4; check "closed" "d['forge']['openMr'] is None"
key Control+b; pause 0.3; shot forge-narrow; key Control+b; pause 0.3
echo "Create MR: the flyout, as draft, the target's picker"
clickon "side:refs/heads/feature/new" right; pause 0.3; q "[r for r in d['menu'][0] if 'merge request' in r.lower()]"
P=$(row_at "merge request"); [ -n "$P" ] && ri move $P -- wait 40 -- click; pause 1
q "d['forge']['create'] and d['forge']['create']['draft'], sorted(k for k in d['anchors'] if k.startswith('create-'))"; shot forge-create
key Escape; pause 0.5
reqs | python3 -c "
import json,sys
r=json.load(sys.stdin); r=r if isinstance(r,list) else r.get('requests',r)
print([ (x.get('method'), x.get('path',x.get('url',''))[-60:]) for x in r if x.get('method')!='GET'])"
finish
