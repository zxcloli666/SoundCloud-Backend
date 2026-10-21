#!/bin/sh
set -eu
host=$1
exec ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" \
  "docker exec \$(docker ps -q --filter label=com.docker.compose.service=chat-api) sh -c 'curl -fsS -H \"x-chat-admin-token: \$(cat /run/secrets/chat/chat-admin-token)\" http://127.0.0.1:\${PORT:-3000}/chat-admin/metrics'"
