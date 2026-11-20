#!/usr/bin/env bash
# A screenshot of one process's window and nothing else, found by its pid: a script's `shot` line (bench.rs).
# usage: winshot.sh <pid> <out.png>
pid=$1; out=$2
for id in $(xprop -root _NET_CLIENT_LIST 2>/dev/null | grep -o '0x[0-9a-f]*'); do
  if [ "$(xprop -id "$id" _NET_WM_PID 2>/dev/null | grep -o '[0-9]*$')" = "$pid" ]; then exec import -window "$id" "$out"; fi
done
echo "no window for pid $pid" >&2
