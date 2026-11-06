#!/usr/bin/env bash
# The Merge split button on GitHub, against the harness's fake forge: its height, its divider, its hover.
source "$(dirname "$0")/lib.sh"
export GITVOLT_E2E_PORT_BASE=${GITVOLT_E2E_PORT_BASE:-7443}
LAB="$TOOLS/forge-lab.sh"
"$LAB" start >/dev/null
trap '"$LAB" stop' EXIT
curl -sf -X POST "http://127.0.0.1:$GITVOLT_E2E_PORT_BASE/test/reset" >/dev/null
export GITVOLT_TEST_FORGE=$("$LAB" base)
G=$("$LAB" fixture flow-gh github.com octo-org/widget | tail -1)
clean_app
up "$G"; until_q "d['rows'] and d['rows'] >= 2" 8; pause 1
key Control+comma; pause 0.4; clickon settings-nav:accounts; pause 0.5
clickon account-add-first; pause 0.3; ri type "ghp_FAKE-e2e-octocat"; pause 0.2; clickon account-submit; pause 1.5
key Escape; pause 2
q "d['forge']['me'], sorted(k for k in d['anchors'] if k.startswith('side:mr'))"
clickon "side:mr:3"; pause 2
shot merge-view
q "d['anchors'].get('mr-merge-go'), d['anchors'].get('mr-merge-method'), d['anchors'].get('mr-merge-act')"
check "the chevron is as tall as Merge" "d['anchors']['mr-merge-method'][3] == d['anchors']['mr-merge-act'][3]"
read -r MX MY MW MH <<<"$(q "(lambda b: '%d %d %d %d' % (b[0], b[1], b[2], b[3]))(d['anchors']['mr-merge-act'])")"
shot merge-btn $((MX - 8)) $((MY - 8)) $((MW + 16)) $((MH + 16))
moveon mr-merge-method; pause 0.3
shot merge-btn-hover $((MX - 8)) $((MY - 8)) $((MW + 16)) $((MH + 16))
finish
