#!/usr/bin/env bash
# A nested compositor held open, to drive a command at a time while finding out how a flow should go:
#   timeout 1800 tools/realinput.sh - bash tools/flows/session.sh <env-file> <seconds>     (in the background), then in each command
#   source <env-file>; source tools/flows/lib.sh; ...
{ export -p; declare -fx; } >"$1"
sleep "${2:-1800}"
