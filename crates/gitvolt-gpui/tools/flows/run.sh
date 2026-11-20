#!/usr/bin/env bash
# Runs the real-input flows, each in a nested compositor of its own: flows/run.sh [name...], from the repository's root. By default,
# all. A flow's first lines can say `# native: yes` (run again on Wayland) or `# layout: de`. NATIVE=1 runs only on Wayland.
# The memory cap is realinput.sh's; a flow past its time (FLOW_SECS, 300 s) fails.
set -u
# In a capped scope of its own (`capped.sh`), what it starts with it: a kill stays there.
grep -q '/run-[^/]*\.scope$' /proc/self/cgroup || exec "$(dirname "$0")/../capped.sh" "$0" "$@"
here=$(cd "$(dirname "$0")" && pwd)
names=("$@")
[ ${#names[@]} = 0 ] && names=($(ls "$here" | grep -E '^[0-9a-z].*\.sh$' | grep -vE '^(lib|run|session|monkey)\.sh$' | sed 's/\.sh$//'))
bad=0
for n in "${names[@]}"; do
  # The machine is shared: below 2.5 GB available a flow waits, up to half an hour.
  for _ in $(seq 1 60); do [ "$(free -m | awk 'NR==2{print $7}')" -ge 2560 ] && break; sleep 30; done
  f=$here/$n.sh
  layout=$(sed -n 's/^# layout: //p' "$f" | head -1)
  modes=(x11)
  grep -q '^# native: yes' "$f" && modes=(x11 wayland)
  [ -n "${NATIVE:-}" ] && modes=(wayland)
  for mode in "${modes[@]}"; do
    echo "== $n ($mode${layout:+, $layout})"
    NATIVE=$([ "$mode" = wayland ] && echo 1) LAYOUT=${LAYOUT:-$layout} FLOW_NAME=$n timeout -k 5 "${FLOW_SECS:-300}" "$here/../realinput.sh" - bash "$f"
    code=$?
    [ "$code" = 0 ] || { echo "== $n ($mode) FAILED ($code)"; bad=$((bad + 1)); }
  done
done
echo "$bad flows failed"
[ "$bad" = 0 ]
