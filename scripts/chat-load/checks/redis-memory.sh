#!/bin/sh
set -eu
host=$1
service=$2
max_percent=$3
share=$(ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" \
  "docker exec \$(docker ps -q --filter label=com.docker.compose.service=$service) redis-cli INFO memory" |
  tr -d '\r' | awk -F: '$1 == "used_memory" { u = $2 } $1 == "maxmemory" { m = $2 } END { if (m > 0) printf "%d\n", u * 100 / m }') || exit 2
[ -n "$share" ] || exit 2
echo "$service memory ${share}%"
[ "$share" -le "$max_percent" ] || exit 1
