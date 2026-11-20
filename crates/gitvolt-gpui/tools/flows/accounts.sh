#!/usr/bin/env bash
# Several forge accounts on a host and a repository's choice, by real input, against the harness's fake forge on 127.0.0.1 only: its fictional users,
# a second token added to its seed, a file token store in the isolated data folder. Never a token in what's checked or printed.
source "$(dirname "$0")/lib.sh"
export GITVOLT_E2E_PORT_BASE=${GITVOLT_E2E_PORT_BASE:-7445}
LAB="$TOOLS/forge-lab.sh"
"$LAB" start >/dev/null
trap '"$LAB" stop' EXIT
H=http://127.0.0.1:$GITVOLT_E2E_PORT_BASE
curl -sf -X POST "$H/test/reset" >/dev/null
export GITVOLT_TEST_FORGE=$("$LAB" base)
HOST=gitlab.example.com
ADA=glpat-FAKE-e2e-ada
GRACE=glpat-FAKE-e2e-grace-work
# The seed's read-only Grace, given a token that can write.
curl -sf "$H/test/forge/seed" | python3 -c "
import json, sys
s = json.load(sys.stdin)
t = s['gitlab']['tokens']
g = next(x for x in t if x['user']['username'] == 'grace')
t.append({**g, 'token': '$GRACE', 'scopes': ['api', 'read_user']})
print(json.dumps(s))" | curl -sf -X POST -H 'content-type: application/json' -d @- "$H/test/forge/seed" >/dev/null
G=$("$LAB" fixture flow-acc "$HOST" group/project | tail -1)
clean_app
up "$G"; until_q "d['rows'] and d['rows'] >= 2" 8; pause 0.5
accounts() { key Control+comma; pause 0.4; clickon settings-nav:accounts; pause 0.5; }
repository() { key Control+comma; pause 0.4; clickon settings-nav:repository; pause 0.8; }
ids() { echo "[a['id'] for a in d['accounts']]"; }
add_token() { ri type "$1"; pause 0.2; clickon account-submit; pause 1.2; }

echo "the first account: the host's, by its name"
accounts
clickon account-add-first; pause 0.3
check "Sign in as first" "'account-submit' in d['anchors']"
add_token "$ADA"
until_q "d['accounts'] and len(d['accounts']) == 1" 5
check "its id is the host, no default marked with nothing to choose" "$(ids) == ['$HOST'] and 'account-default:$HOST' not in d['anchors']"

echo "another user's token on the same host adds a row"
clickon account-add-more; pause 0.4
check "the form offers to replace Ada's" "'account-taken-replace' in d['anchors']"
shot acc-add
check "Add isn't refused for a taken host" "'account-submit' in d['anchors']"
add_token "$GRACE"
until_q "d['accounts'] and len(d['accounts']) == 2" 5
SECOND=$(q "d['accounts'][1]['id']")
check "two, the second named by its user" "d['accounts'][0]['id'] == '$HOST' and '$SECOND'.startswith('$HOST#') and [a['username'] for a in d['accounts']] == ['ada', 'grace']"
check "the host's first is marked its default" "'account-default:$HOST' in d['anchors']"
shot acc-two

echo "the same user's token updates theirs: no row, and it says so"
clickon account-add-more; pause 0.4
add_token "$ADA"
until_q "d['accountsNotice'] == \"Updated @ada's token\"" 5
check "still two" "len(d['accounts']) == 2"
shot acc-updated
clickon account-add-more; pause 0.3
check "the next action clears the notice" "d['accountsNotice'] is None"
clickon account-cancel; pause 0.3

echo "Replace token takes only that user's"
clickon "account-replace:$SECOND"; pause 0.4
shot acc-replace
add_token "$ADA"
check "another user's token is refused, the form stays" "'account-submit' in d['anchors'] and len(d['accounts']) == 2"
shot acc-replace-refused
clickon account-cancel; pause 0.3
key Escape; pause 0.3

echo "Automatic: the host's default"
until_q "d['forge']['accountId'] == '$HOST'" 8
check "Ada's the repository's" "d['forge']['me'] == 'ada'"

echo "the repository picks the second account"
repository
check "the row's there with two to choose from" "'setting-row:forgeAccount' in d['anchors'] and d['repoAccounts'] == ['$HOST', '$SECOND']"
shot acc-repo
clickon setting:forgeAccount; pause 0.3
q "[r['label'] for r in d['menuRows']]"
check "Automatic names its default" "[r['label'] for r in d['menuRows']] == ['Automatic (@ada)', '@ada', '@grace']"
pick "@grace"; pause 0.8
key Escape; pause 0.3
until_q "d['forge']['accountId'] == '$SECOND'" 8
check "Grace is who the views act as" "d['forge']['me'] == 'grace'"
check_sh "chosen in the profile, by its id" "grep -q '\"forgeAccount\": *\"$SECOND\"' '$REALINPUT_ROOT/app/config/gitvolt-gpui/profiles/default/profile.json' || sleep 1 && grep -q '\"forgeAccount\": *\"$SECOND\"' '$REALINPUT_ROOT/app/config/gitvolt-gpui/profiles/default/profile.json'"

echo "over a restart"
stop_app
up; until_q "d['rows'] and d['rows'] >= 2" 8
until_q "d['forge']['accountId'] == '$SECOND'" 8
check "the choice stayed" "d['forge']['me'] == 'grace'"
repository
check "and the row says so" "'setting-row:forgeAccount' in d['anchors']"
shot acc-repo2
key Escape; pause 0.3

echo "removing the default says what else changes, and it's gone"
accounts; pause 0.4
clickon "account-remove:$HOST"; pause 0.4
shot acc-remove-armed
check "armed in place" "d['armed']"
clickon "account-remove:$HOST"; pause 1
until_q "len(d['accounts']) == 1" 5
check "Grace keeps her name" "$(ids) == ['$SECOND']"
check "nothing to mark now" "'account-default:$SECOND' not in d['anchors']"
key Escape; pause 0.3
check "Esc closes Settings though the pressed button's gone" "not d['settings']['open']"
until_q "d['forge']['accountId'] == '$SECOND'" 8

echo "with one account left the row stays while it's chosen, to go back to Automatic"
repository
check "chosen: there" "'setting-row:forgeAccount' in d['anchors']"
clickon setting:forgeAccount; pause 0.3; pick "Automatic"; pause 0.8
until_q "'setting-row:forgeAccount' not in d['anchors']" 5
key Escape; pause 0.3

echo "tokens are nowhere they shouldn't be"
check_sh "not in the app's log" "! grep -q 'glpat-FAKE' '$REALINPUT_ROOT/app.log'"
check_sh "not in the profile" "! grep -rq 'glpat-FAKE' '$REALINPUT_ROOT/app/config'"
check_sh "not in what the app says of itself" "! grep -q 'glpat-FAKE' '$GITVOLT_DEBUG_PIPE.out'"
stop_app
finish
