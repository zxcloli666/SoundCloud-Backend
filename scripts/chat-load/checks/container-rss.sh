#!/bin/sh
set -eu
host=$1
service=$2
max_mb=$3
used=$(ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" \
  "docker stats --no-stream --format '{{.MemUsage}}' \$(docker ps -q --filter label=com.docker.compose.service=$service)" |
  awk '{ v = $1; u = v; gsub(/[0-9.]/, "", u); gsub(/[^0-9.]/, "", v);
         m = (u == "GiB") ? 1024 : (u == "MiB") ? 1 : (u == "KiB") ? 1 / 1024 : (u == "GB") ? 1000 : (u == "MB") ? 1 : 0;
         printf "%d\n", v * m }') || exit 2
[ -n "$used" ] || exit 2
echo "$service rss ${used} MB"
[ "$used" -le "$max_mb" ] || exit 1
