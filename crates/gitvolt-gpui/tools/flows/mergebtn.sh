#!/usr/bin/env bash
# The Merge split button on GitHub, against the harness's fake forge: its height, its divider, its hover, and
# its armed label, which grows left inside the view and leaves the chevron beside it; a PR's hover card with a long title
# over the open view.
source "$(dirname "$0")/lib.sh"
export GITVOLT_E2E_PORT_BASE=${GITVOLT_E2E_PORT_BASE:-7443}
LAB="$TOOLS/forge-lab.sh"
"$LAB" start >/dev/null
trap '"$LAB" stop' EXIT
curl -sf -X POST "http://127.0.0.1:$GITVOLT_E2E_PORT_BASE/test/reset" >/dev/null
export GITVOLT_TEST_FORGE=$("$LAB" base)
G=$("$LAB" fixture flow-gh github.com octo-org/widget | tail -1)
# #6 with a title too long for a card's line.
"$LAB" seed | python3 -c '
import json, sys
seed = json.load(sys.stdin)
for p in seed["github"]["pulls"]:
    if p["number"] == 6:
        p["title"] = "Explore caching for the widget index without the per-user keys, and drop the old warmers"
json.dump(seed, sys.stdout)' >"$TMPD/gh-seed.json"
curl -sf -X POST -H 'content-type: application/json' --data @"$TMPD/gh-seed.json" "http://127.0.0.1:$GITVOLT_E2E_PORT_BASE/test/forge/seed" >/dev/null || fail "the seed was refused"
clean_app
up "$G"; until_q "d['rows'] and d['rows'] >= 2" 8; pause 1
key Control+comma; pause 0.4; clickon settings-nav:accounts; pause 0.5
clickon account-add-first; pause 0.3; ri type "ghp_FAKE-e2e-octocat"; pause 0.2; clickon account-submit; pause 1.5
key Escape; pause 2
q "d['forge']['me'], sorted(k for k in d['anchors'] if k.startswith('side:mr'))"
clickon "side:mr:3"; pause 2
shot merge-view
q "d['anchors'].get('mr-merge-go'), d['anchors'].get('mr-merge-method'), d['anchors'].get('mr-merge-buttons')"
check "the chevron is as tall as Merge" "d['anchors']['mr-merge-method'][3] == d['anchors']['mr-merge-act'][3]"
read -r MX MY MW MH <<<"$(q "(lambda b: '%d %d %d %d' % (b[0], b[1], b[2], b[3]))(d['anchors']['mr-merge-buttons'])")"
shot merge-btn $((MX - 8)) $((MY - 8)) $((MW + 16)) $((MH + 16))
moveon mr-merge-method; pause 0.3
shot merge-btn-hover $((MX - 8)) $((MY - 8)) $((MW + 16)) $((MH + 16))
echo "armed: the label grows left over the row, inside the view, the chevron beside it"
clickon mr-merge-act; pause 0.4
check "armed" "d['armed'] and d['armOverlay']"
check "it keeps Merge's right edge" "abs(d['armOverlay'][0] + d['armOverlay'][2] - d['anchors']['mr-merge-act'][0] - d['anchors']['mr-merge-act'][2]) < 1"
check "the chevron stays uncovered" "d['armOverlay'][0] + d['armOverlay'][2] <= d['anchors']['mr-merge-method'][0] + 0.5"
check "inside the view" "d['armOverlay'][0] >= d['anchors']['arm-bounds:mr-body'][0] - 0.5"
shot merge-btn-armed $((MX - 300)) $((MY - 8)) $((MW + 316)) $((MH + 16))
key Escape; pause 0.3
check "Esc disarmed it, nothing merged" "not d['armed'] and 'merged' not in (d['toast'] or '').lower()"
echo "#6's card, beside the sidebar, over the open view: its title wraps inside it"
moveon "side:mr:6"; pause 1.2
shot merge-card
finish
