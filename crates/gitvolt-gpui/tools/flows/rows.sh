#!/usr/bin/env bash
# native: yes
# Submodules' rows by real input.
source "$(dirname "$0")/lib.sh"
clean_app
SUB=$WORK/flows/rows-sub; rm -rf "$SUB"; mkdir -p "$SUB"; g "$SUB" init -q -b main; g "$SUB" config user.name 'Ada Fixture'; g "$SUB" config user.email ada@example.com; echo s >"$SUB/s.txt"; g "$SUB" add .; g "$SUB" commit -qm "Sub base"
A=$(small_repo rows)
g "$A" -c protocol.file.allow=always submodule --quiet add "$SUB" libs/sub; g "$A" commit -qm "Add a submodule"
g "$A" config protocol.file.allow always
GIT_ALLOW_PROTOCOL=file:ssh; export GIT_ALLOW_PROTOCOL
up "$A"; until_q "d['rows'] and d['rows'] >= 7" 5; pause 0.8
K=$(q "[k for k in d['anchors'] if k.startswith('side:') and 'sub' in k and not k.startswith('side:refs')][:1]"); echo "panel rows: $K"
R=$(q "at([k for k in d['anchors'] if k.startswith('side:') and 'sub' in k and not k.startswith('side:refs')][0])"); echo "at $R"
ri move $R -- wait 60 -- click right -- wait 300; q "d['menu'][0]"; shot rows-sub
pick "Update"; pause 1.2; q "d['toast'], d['running']"
key Control+p; pause 0.3; ri type ">Sync URLs"; pause 0.4; key Return; pause 1.2; q "d['toast']"; check "Sync URLs by the palette" "d['toast'] and 'ync' in d['toast']"
check_sh "the submodule's still checked out" "[ -f '$A/libs/sub/s.txt' ]"
g "$A" submodule --quiet deinit -f libs/sub; pause 1.2
ri move $R -- wait 60 -- click right -- wait 300; q "d['menu'][0]"; pick "Initialise"; pause 1.5; q "d['toast']"; shot rows-init
check_sh "Initialise brought it back" "[ -f '$A/libs/sub/s.txt' ]"
ri move $R -- wait 60 -- click right -- wait 300; pick "Remove"; pause 0.4; q "d['armed']"; shot rows-sub-rm; pick "Remove"; pause 1.5
check_sh "Remove took it out" "! git -C '$A' config -f .gitmodules --get submodule.libs/sub.path"
finish
