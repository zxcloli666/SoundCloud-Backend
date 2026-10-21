#!/bin/sh
set -eu
host=$1
backend=$2
queue=$(ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" "echo 'show stat' | socat stdio /run/haproxy/admin.sock" |
  awk -F, -v name="$backend" 'NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; next } $1 == name && $2 == "BACKEND" { print $col["qcur"] }') || exit 2
[ -n "$queue" ] || exit 2
echo "$host $backend queue $queue"
[ "$queue" -eq 0 ] || exit 1
