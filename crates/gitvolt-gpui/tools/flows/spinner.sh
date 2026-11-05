#!/usr/bin/env bash
# native: yes
# What a turning spinner costs: a fetch that hangs, the CPU it takes meanwhile, and its Cancel in the status bar.
source "$(dirname "$0")/lib.sh"
clean_app
H=$(small_repo hung)
printf '#!/bin/sh\nsleep 600\n' >"$WORK/flows/hang-ssh.sh"; chmod +x "$WORK/flows/hang-ssh.sh"
g "$H" remote add origin ssh://git@forge.example.com/hung.git; g "$H" config core.sshCommand "$WORK/flows/hang-ssh.sh"
up "$H"; until_q "d['rows'] and d['rows'] >= 6" 5; pause 1
main_cpu() { python3 - "$APP_PID" "$1" <<'PY'
import sys, time
pid, secs = sys.argv[1], float(sys.argv[2])
read = lambda: sum(int(x) for x in open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()[11:13])
a = read(); time.sleep(secs); print(round((read() - a) / secs, 1))
PY
}
IDLE=$(main_cpu 5); echo "idle: $IDLE % of a core"
check_sh "idle costs nothing ($IDLE)" "python3 -c 'import sys; sys.exit(0 if $IDLE < 1 else 1)'"
clickon tb-sync; pause 0.5; ri move 800 600 -- wait 300
check "the fetch runs" "d['running']"
SPIN=$(main_cpu 8); echo "spinning: $SPIN % of a core"
check_sh "a spinner costs little ($SPIN)" "python3 -c 'import sys; sys.exit(0 if $SPIN < 8 else 1)'"
shot spinner 0 60 1600 50
echo "the hung fetch's Cancel"
P=$(q "(lambda h: '')(0)")
key Control+comma; pause 0.3; key Escape; pause 0.3
Y=$(q "round(d['window']['h'] - 10)"); clickxy 301 "$Y"; pause 1
until_q "not d['running']" 5
finish
