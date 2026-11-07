#!/usr/bin/env bash
# A picture of the app's window on a nested display (tools/realinput.sh).
[ "${DISPLAY:-}" = "${GITVOLT_NESTED_DISPLAY:-none}" ] || { echo "not a nested display"; exit 1; }
win=$(xprop -root _NET_CLIENT_LIST | grep -o '0x[0-9a-f]*' | tail -1)
import -window "$win" "$1"
