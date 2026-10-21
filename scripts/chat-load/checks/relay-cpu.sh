#!/bin/sh
set -eu
host=$1
max=$2
busy=$(ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" '
read -r _ a b c d e f g h _ < /proc/stat
sleep 5
read -r _ A B C D E F G H _ < /proc/stat
total=$((A + B + C + D + E + F + G + H - a - b - c - d - e - f - g - h))
idle=$((D + E - d - e))
echo $(((total - idle) * 100 / total))
') || exit 2
echo "$host cpu ${busy}%"
[ "$busy" -le "$max" ] || exit 1
