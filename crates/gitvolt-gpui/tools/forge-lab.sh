#!/usr/bin/env bash
# The fake forge for this build and its reference, on 127.0.0.1 only: `gitvolt-harness serve`, which also
# serves the web UI's WebSocket (`tools/forgeref.mjs`). Nothing here can reach a real host: the repositories
# are fixtures under target/bench/work/forge, whose git may only use local paths.
#   forge-lab.sh start                      starts the harness (port 7433); `stop` ends that exact pid
#   forge-lab.sh base                       the fake forge's address, for GITVOLT_TEST_FORGE
#   forge-lab.sh fixture <name> <forge> <project> [harness fixture]
#                                           a fixture repository whose origin is named by its forge URL and read locally
#   forge-lab.sh profile <name>             a clean XDG root for this build (the settings as above), printed
#   forge-lab.sh account <host> <kind> <token>   adds an account to the harness's own Api (the reference's)
#   forge-lab.sh requests|seed              the fake forge's log, or its state
set -eu
root=$(pwd)/target/bench/work/forge
port=${GITVOLT_E2E_PORT_BASE:-7433}
http=http://127.0.0.1:$port
pidfile=$root/harness.pid
case "${1:-}" in
  start)
    mkdir -p "$root/web-config"
    # Its settings before it first starts: no fetches, no update checks, nothing from Gravatar.
    [ -f "$root/web-config/settings.json" ] || echo '{"version":2,"gravatar":false,"updateCheck":false,"fetchIntervalSecs":0}' >"$root/web-config/settings.json"
    if [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then echo "already running"; exit 0; fi
    ( ulimit -v 8000000; ulimit -c 0; exec nohup target/bench/release/gitvolt-harness serve --port "$port" --fixture-root "$root" --config-dir "$root/web-config" >"$root/harness.log" 2>&1 ) &
    echo $! >"$pidfile"
    for _ in $(seq 1 100); do curl -sf "$http/health" >/dev/null && break; sleep 0.1; done
    curl -sf "$http/test/forge/base"; echo
    ;;
  stop)
    if [ -f "$pidfile" ]; then kill "$(cat "$pidfile")" 2>/dev/null || true; rm -f "$pidfile"; fi
    ;;
  base) curl -sf "$http/test/forge/base"; echo ;;
  fixture)
    name=$2; forge=$3; project=$4; kind=${5:-sync}
    dir=$root/$name
    repo=$(target/bench/release/gitvolt-harness fixture "$kind" "$dir")
    origin=$(dirname "$repo")/origin.git
    git -C "$repo" remote set-url origin "https://$forge/$project.git"
    # Fetches and pushes go to the fixture's own bare origin: "ssh" to the forge's host runs this script (the forge stays
    # the URL's host, as the e2e does), and git may use nothing else.
    cat >"$dir/fake-ssh.sh" <<SH
#!/bin/sh
for a; do last=\$a; done
case "\$last" in
  git-upload-pack*) exec git upload-pack "$origin" ;;
  git-receive-pack*) exec git receive-pack "$origin" ;;
esac
exit 1
SH
    chmod +x "$dir/fake-ssh.sh"
    git -C "$repo" config "url.ssh://git@$forge/.insteadOf" "https://$forge/"
    git -C "$repo" config core.sshCommand "$dir/fake-ssh.sh"
    git -C "$repo" config protocol.allow never
    git -C "$repo" config protocol.ssh.allow always
    git -C "$repo" config protocol.file.allow always
    echo "$repo"
    ;;
  profile)
    # An isolated profile for this build, clean each time: `GITVOLT_XDG=$(forge-lab.sh profile name) tools/run.sh ...`
    xdg=$root/xdg-$2
    rm -rf "$xdg"; mkdir -p "$xdg/config/gitvolt-gpui" "$xdg/data" "$xdg/cache"
    echo '{"version":2,"gravatar":false,"updateCheck":false,"fetchIntervalSecs":0}' >"$xdg/config/gitvolt-gpui/settings.json"
    echo "$xdg"
    ;;
  account)
    curl -sf -X POST "$http/test/forge/account" -H 'content-type: application/json' -d "{\"host\":\"$2\",\"kind\":\"$3\",\"token\":\"$4\"}" >/dev/null
    ;;
  requests) curl -sf "$http/test/forge/requests" ;;
  seed) curl -sf "$http/test/forge/seed" ;;
  reset) curl -sf -X POST "$http/test/reset" ;;
  *) echo "usage: forge-lab.sh start|stop|base|fixture|account|requests|seed|reset" >&2; exit 2 ;;
esac
