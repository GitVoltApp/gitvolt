#!/usr/bin/env bash
# Runs a command in a transient systemd scope of its own, with a memory cap and first in line for the OOM killer: a kill stays inside
# that scope, so it can't stop the session it was started from (a scope with OOMPolicy=stop would). Every build, test, app, compositor
# and reference goes through this. Never runs a command uncapped: no systemd-run, or one that fails, stops here.
# usage: capped.sh <command...>   GITVOLT_CAP: MemoryMax (10G).
set -u
[ $# -gt 0 ] || { echo "capped.sh: no command" >&2; exit 2; }
# Already in such a scope: no nesting.
if grep -q '/run-[^/]*\.scope$' /proc/self/cgroup 2>/dev/null; then
  exec choom -n 1000 -- "$@"
fi
command -v systemd-run >/dev/null || { echo "capped.sh: no systemd-run: not running $1 uncapped" >&2; exit 127; }
# Exec all the way down, so a timeout or a kill of this pid reaches the command; if systemd-run can't start a scope it says so and fails.
exec systemd-run --user --scope --quiet --collect -p OOMPolicy=continue -p "MemoryMax=${GITVOLT_CAP:-10G}" -p MemorySwapMax=2G choom -n 1000 -- "$@"
