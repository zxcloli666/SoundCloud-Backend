#!/bin/sh
set -eu
host=$1
exec ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" \
  "docker exec \$(docker ps -q --filter label=com.docker.compose.service=backend) sh -c 'curl -fsS -H \"x-admin-token: \$ADMIN_TOKEN\" http://127.0.0.1:\${PORT:-3000}/admin/metrics'"
