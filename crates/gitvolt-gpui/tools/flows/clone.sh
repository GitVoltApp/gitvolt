#!/usr/bin/env bash
# A clone that hangs, and its Cancel: git here is a wrapper whose `clone` sleeps, everything else the real one. The URL is a fictional host that nothing ever connects to.
source "$(dirname "$0")/lib.sh"
clean_app
real=$(command -v git)
rm -f "$REALINPUT_ROOT/bin/git"
printf '#!/bin/sh\nfor a; do [ "$a" = clone ] && { echo "Cloning into fake..." >&2; exec sleep 600; }; done\nexec "%s" "$@"\n' "$real" >"$REALINPUT_ROOT/bin/git"; chmod +x "$REALINPUT_ROOT/bin/git"
A=$(small_repo cl); D=$WORK/flows/cl-target; rm -rf "$D"
up "$A"; until_q "d['rows']" 5
key Control+o; pause 0.5; shot clone-open
clickon open-url; pause 0.2; ri type "https://git.example.com/group/fake.git"; pause 0.3
clickon open-dest; pause 0.2; key Control+a; ri type "$D"; pause 0.3; shot clone-typed
clickon open-clone; pause 1; shot clone-running
q "sorted(k for k in d['anchors'] if 'cancel' in k.lower() or 'clone' in k.lower())"
clickon clone-cancel; pause 1; shot clone-cancelled
check "the screen's still up, no tab came" "d['openScreen'] is not None and len(d['tabs']) == 2"
check_sh "nothing's left of the destination" "[ ! -e '$D' ]"
finish
