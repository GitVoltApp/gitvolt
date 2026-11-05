#!/usr/bin/env bash
# native: yes
# Starts that aren't the usual one: the system title bar, git too old, the Open screen's banner, a minimized window's pause.
source "$(dirname "$0")/lib.sh"
clean_app
A=$(small_repo start2)
echo "the Open screen's default-directory banner"
mkdir -p "$REALINPUT_ROOT/app/home/Coding/fixture-repo"
up; until_q "d['openScreen'] is not None" 5; pause 0.5
shot st-open
q "sorted(k for k in d['anchors'] if 'banner' in k or 'open' in k.lower())[:20]"
q "d['openScreen']"
clickon open-directory; pause 1
check "Open directory with no portal: the app's alive" "True"
q "d['toast'], d['openScreen']['error'], d['modals']"; shot st-nopick
clickon open-choose; pause 1
q "d['toast'], d['openScreen']['error']"
stop_app

echo "the system title bar"
python3 - "$REALINPUT_ROOT/app/config/gitvolt-gpui/settings.json" <<'PY'
import json, sys
p = sys.argv[1]; v = json.load(open(p)); v["systemTitleBar"] = True; json.dump(v, open(p, "w"))
PY
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5; pause 0.5
check "no window buttons of its own" "'win-close' not in d['anchors']"
check "the hamburger moved" "'hamburger' in d['anchors']"
shot st-system
stop_app
clean_app

echo "git too old"
FAKE=$WORK/flows/old-git; mkdir -p "$FAKE"
cat >"$FAKE/git" <<'SH'
#!/bin/sh
for a; do case "$a" in --version|version) echo "git version 2.1.0"; exit 0 ;; esac; done
exec /usr/bin/git "$@"
SH
chmod +x "$FAKE/git"
rm -f "$REALINPUT_ROOT/bin/git"; ln -s "$FAKE/git" "$REALINPUT_ROOT/bin/git"
up "$A"; pause 1.5
shot st-oldgit
q "d['tabs'], d['toast']"
stop_app
rm -f "$REALINPUT_ROOT/bin/git"; ln -s /usr/bin/git "$REALINPUT_ROOT/bin/git"

echo "a minimized window: no background fetch runs; it comes back on restore"
O=$WORK/flows/start2-origin.git; rm -rf "$O"; git init -q --bare "$O"; g "$A" remote add origin "$O" 2>/dev/null; g "$A" config protocol.file.allow always
g "$A" config core.sshCommand /bin/false
echo '{"version":2,"gravatar":false,"updateCheck":false,"fetchIntervalSecs":1}' >"$REALINPUT_ROOT/app/config/gitvolt-gpui/settings.json"
up "$A"; until_q "d['rows'] and d['rows'] >= 6" 5; pause 1
check "visible" "d['window']['visible']"
clickon win-minimize; pause 0.6
check "minimized: hidden, not active" "not d['window']['active']"
q "d['window']['visible'], d['settings']['fetchSecs']"
launch_app "$A"; pause 0.8
check "a later launch brings it back" "d['window']['visible'] and d['window']['active']"
stop_app
finish
