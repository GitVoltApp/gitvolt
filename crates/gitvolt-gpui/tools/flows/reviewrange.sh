#!/usr/bin/env bash
# A range of lines picked by a drag from the "+" in review mode, against the harness's fake forge; the Merge split button;
# Submit review's box under the chip, typed into.
source "$(dirname "$0")/lib.sh"
export GITVOLT_E2E_PORT_BASE=${GITVOLT_E2E_PORT_BASE:-7443}
LAB="$TOOLS/forge-lab.sh"
"$LAB" start >/dev/null
trap '"$LAB" stop' EXIT
curl -sf -X POST "http://127.0.0.1:$GITVOLT_E2E_PORT_BASE/test/reset" >/dev/null
export GITVOLT_TEST_FORGE=$("$LAB" base)
G=$("$LAB" fixture flow-rr gitlab.example.com group/project | tail -1)
# !12's branch brings a file of some lines.
g "$G" config user.name 'Ada Fixture'; g "$G" config user.email ada@example.com
g "$G" fetch -q origin; g "$G" switch -q dev
for i in $(seq 1 30); do echo "review line $i"; done >"$G/range.txt"
g "$G" add range.txt; g "$G" commit -qm 'Add the range file'; g "$G" push -q origin dev; g "$G" fetch -q origin
g "$G" switch -q main
# !12's head is origin/dev's tip, GitLab's /diffs for it this compare's own, as the e2e seeds them.
BASE=$(g "$G" merge-base origin/main origin/dev); HEAD=$(g "$G" rev-parse origin/dev); MAIN=$(g "$G" rev-parse origin/main)
FILE=range.txt
g "$G" diff -U3 "$BASE" "$HEAD" -- "$FILE" >"$TMPD/rr.patch"
"$LAB" seed | python3 -c '
import json, sys
seed = json.load(sys.stdin)
head, main, file, patch_path = sys.argv[1:5]
patch = open(patch_path).read()
diff = "\n".join(patch.split("\n")[patch.split("\n").index(next(l for l in patch.split("\n") if l.startswith("@@"))):])
for m in seed["gitlab"]["mergeRequests"]:
    if m["iid"] == 12:
        m.update(headSha=head, baseSha=main, diffs=[{"oldPath": file, "newPath": file, "diff": diff, "newFile": False, "renamedFile": False, "deletedFile": False}])
json.dump(seed, sys.stdout)' "$HEAD" "$MAIN" "$FILE" "$TMPD/rr.patch" >"$TMPD/rr-seed.json"
curl -sf -X POST -H 'content-type: application/json' --data @"$TMPD/rr-seed.json" "http://127.0.0.1:$GITVOLT_E2E_PORT_BASE/test/forge/seed" >/dev/null || fail "the seed was refused"
clean_app
up "$G"; until_q "d['rows'] and d['rows'] >= 2" 8; pause 1
key Control+comma; pause 0.4; clickon settings-nav:accounts; pause 0.5
clickon account-add-first; pause 0.3; ri type "glpat-FAKE-e2e-ada"; pause 0.2; clickon account-submit; pause 1.2
key Escape; pause 2
until_q "d['forge']['me'] == 'ada'" 6
clickon "side:mr:12"; pause 1.5
check "the MR opened" "d['forge']['openMr'] == 12"
clickon mr-compare; pause 2.5
shot rr-compare
q "[k for k in d['anchors'] if 'file' in k or k.startswith('tip:$FILE')][:12]"
clickon "tip:$FILE"; until_q "d['diff'] and d['diff']['path'] == '$FILE'" 6; pause 0.8
shot rr-diff
read -r RY RX <<<"$(q "(lambda b: '%d %d' % (b[1], b[0]))(d['anchors']['diff-ruler'])")"
echo "a drag from the + over three lines, past a card's or the pane's edge"
ri move $((RX - 300)) $((RY + 9 + 18)) -- wait 300
q "[k for k in d['anchors'] if k.startswith('rv-')]"
PLUS=$(at rv-plus-new); [ -n "$PLUS" ] || fail "no + on the line"
[ -n "$PLUS" ] && ri move $PLUS -- wait 120 -- down -- wait 80 -- move $((RX - 300)) $((RY + 9 + 18 * 3)) -- wait 150
check "the drag is on" "d['diff'] is not None"
shot rr-dragging
ri up -- wait 500
shot rr-box
ri move 500 800 -- wait 200
shot rr-box2
echo "Add to review, then Submit review under the chip: its box takes typing"
ri move $((RX - 300)) $((RY + 9 + 18 * 2)) -- wait 100; ri type "a note on the range"; pause 0.3
ADD=$(q "next((k for k in d['anchors'] if k.startswith('rv-add:')), '')")
[ -n "$ADD" ] && clickon "$ADD"; until_q "d['forge']['review'] and d['forge']['review']['drafts'] >= 1" 5
check "a pending comment" "d['forge']['review']['drafts'] >= 1"
key Control+Alt+r; pause 0.6
check "Submit review is open" "d['reviewSubmit'] is not None"
ri type "the summary"; pause 0.3
shot rr-submit
check "its box took the typing" "d['reviewSubmit']['text'] == 'the summary'"
check "and shows it: the input has a height" "d['anchors']['mr-review-input-pop'][3] >= 40"
key Escape; pause 0.3
check "Esc closed it, nothing sent" "d['reviewSubmit'] is None and d['forge']['review']['drafts'] >= 1"
echo "the same from the chip's menu, then a click into the box"
clickon tb-review-caret; pause 0.4; pick "Submit review…"; pause 0.6
check "Submit review is open from the menu" "d['reviewSubmit'] is not None"
ri type " more"; pause 0.3
check "typing went in" "d['reviewSubmit']['text'] == 'the summary more'"
read -r TX TY <<<"$(q "(lambda b: '%d %d' % (b[0] + 40, b[1] + 20))(d['anchors']['mr-review-text-pop'])")"
ri move "$TX" "$TY" -- wait 60 -- click; pause 0.2; ri type "Z"; pause 0.3
shot rr-submit-menu
check "a click into it, then typing where it put the caret" "'Z' in d['reviewSubmit']['text'] and not d['reviewSubmit']['text'].endswith('Z')"
key Escape; pause 0.3
finish
