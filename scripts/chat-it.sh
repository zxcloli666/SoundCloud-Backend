#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
HERE="$ROOT/scripts/chat-it"
STATE="${CHAT_IT_STATE:-${TMPDIR:-/tmp}/chat-it}"
PG_CONTAINER="${CHAT_IT_PG_CONTAINER:-scd-pg}"
PG_USER="${CHAT_IT_PG_USER:-scd}"
PG_PASSWORD="${CHAT_IT_PG_PASSWORD:-dev}"
PG_PORT="${CHAT_IT_PG_PORT:-5432}"
DB="${CHAT_IT_DB:-scd_chat_it}"
PORT="${CHAT_IT_PORT:-3417}"
FULL_PORT="${CHAT_IT_FULL_PORT:-3418}"
REDIS_RT_PORT="${CHAT_IT_REDIS_RT_PORT:-6392}"
REDIS_PORT="${CHAT_IT_REDIS_PORT:-6393}"
AI_PORT="${CHAT_IT_AI_PORT:-6394}"
RELAY_PORT="${CHAT_IT_RELAY_PORT:-6395}"
S3_PORT="${CHAT_IT_S3_PORT:-6396}"
FRONT_PORT="${CHAT_IT_FRONT_PORT:-6397}"
REDIS_IMAGE="${CHAT_IT_REDIS_IMAGE:-mirror.gcr.io/library/redis:7-alpine}"
PYTHON_IMAGE="${CHAT_IT_PYTHON_IMAGE:-docker.io/library/python:3.12-alpine}"
NGINX_IMAGE="${CHAT_IT_NGINX_IMAGE:-mirror.gcr.io/library/nginx:alpine}"
S3_IMAGE="${CHAT_IT_S3_IMAGE:-docker.io/chrislusf/seaweedfs:4.48}"
S3_BUCKET=chat-it
S3_ACCESS_KEY=chat-it
CONTAINERS=(chat-it-redis-rt chat-it-redis chat-it-ai chat-it-relay chat-it-front chat-it-s3)
TEST_OCK_SECRET=66842626e4763cce2b75991a3e967c29bcc9f02395a3f0115981eb59ab3e4d8e
TEST_OCK_PUB=dde332903df0f9e9fbc91a814131b5bcea28b46f5728e2a7e28700ed4a6c2d4d
DATABASE_URL_IT="postgres://$PG_USER:$PG_PASSWORD@127.0.0.1:$PG_PORT/$DB"

psql_in() { podman exec -i -e PGOPTIONS=--client-min-messages=warning "$PG_CONTAINER" psql -U "$PG_USER" -v ON_ERROR_STOP=1 -qAt "$@"; }

sha384() {
  if command -v sha384sum > /dev/null; then sha384sum "$1"; else shasum -a 384 "$1"; fi | cut -d' ' -f1
}

random_hex() { head -c "$1" /dev/urandom | xxd -p | tr -d '\n'; }

make_keys() {
  local keys="$STATE/keys"
  mkdir -p "$keys"
  chmod 700 "$keys"
  echo "$TEST_OCK_SECRET" | xxd -r -p > "$keys/ock-1"
  head -c 32 /dev/urandom > "$keys/kek-1"
  head -c 32 /dev/urandom > "$keys/reports-kek-1"
  random_hex 24 > "$keys/chat-admin-token"
  random_hex 24 > "$keys/mod-ai-key"
  random_hex 24 > "$keys/s3-secret-1"
  chmod 644 "$keys"/*
  printf '{"identities":[{"name":"chat-it","credentials":[{"accessKey":"%s","secretKey":"%s"}],"actions":["Admin","Read","Write","List","Tagging"]}]}' \
    "$S3_ACCESS_KEY" "$(cat "$keys/s3-secret-1")" > "$STATE/s3.json"
}

migrate() {
  psql_in -d postgres -c "DROP DATABASE IF EXISTS $DB WITH (FORCE)" -c "CREATE DATABASE $DB" < /dev/null
  psql_in -d "$DB" -c "CREATE TABLE _sqlx_migrations (version bigint PRIMARY KEY, description text NOT NULL, installed_on timestamptz NOT NULL DEFAULT now(), success boolean NOT NULL, checksum bytea NOT NULL, execution_time bigint NOT NULL)" < /dev/null
  local file name version description mode
  for file in "$ROOT"/api/migrations/*.sql; do
    name="$(basename "$file" .sql)"
    version=$((10#${name%%_*}))
    description="${name#*_}"
    mode=-1
    head -1 "$file" | grep -q -- '-- no-transaction' && mode=-q
    psql_in -d "$DB" "$mode" -f - < "$file" > /dev/null
    psql_in -d "$DB" -c "INSERT INTO _sqlx_migrations VALUES ($version, '${description//_/ }', now(), true, decode('$(sha384 "$file")', 'hex'), 0)" < /dev/null
  done
}

start_containers() {
  podman rm -f -v "${CONTAINERS[@]}" > /dev/null 2>&1 || true
  podman run -d --name chat-it-redis-rt -p "127.0.0.1:$REDIS_RT_PORT:6379" "$REDIS_IMAGE" redis-server --save '' --appendonly no > /dev/null
  podman run -d --name chat-it-redis -p "127.0.0.1:$REDIS_PORT:6379" "$REDIS_IMAGE" redis-server --save '' --appendonly no > /dev/null
  podman run -d --name chat-it-ai -p "127.0.0.1:$AI_PORT:8080" \
    -v "$HERE/fake-ai.py:/srv/fake-ai.py:ro" -v "$STATE/keys/mod-ai-key:/srv/key:ro" \
    "$PYTHON_IMAGE" python -u /srv/fake-ai.py /srv/key 8080 > /dev/null
  podman run -d --name chat-it-relay -p "127.0.0.1:$RELAY_PORT:8080" \
    -e "CHAT_IT_UPSTREAM=host.containers.internal:$PORT" \
    -v "$HERE/relay.conf.template:/etc/nginx/templates/default.conf.template:ro" \
    "$NGINX_IMAGE" > /dev/null
  podman run -d --name chat-it-front -p "127.0.0.1:$FRONT_PORT:8080" \
    -e "CHAT_IT_CHAT_UPSTREAM=host.containers.internal:$PORT" \
    -e "CHAT_IT_AUTH_UPSTREAM=host.containers.internal:$FULL_PORT" \
    -v "$HERE/front.conf.template:/etc/nginx/templates/default.conf.template:ro" \
    "$NGINX_IMAGE" > /dev/null
  podman run -d --name chat-it-s3 -p "127.0.0.1:$S3_PORT:8333" \
    -v "$STATE/s3.json:/etc/seaweedfs/s3.json:ro" \
    "$S3_IMAGE" server -s3 -s3.config=/etc/seaweedfs/s3.json -dir=/data -master.volumeSizeLimitMB=64 > /dev/null
  make_bucket
}

make_bucket() {
  local answer
  for _ in $(seq 1 60); do
    answer="$(echo "s3.bucket.create -name $S3_BUCKET" | podman exec -i chat-it-s3 weed shell 2>&1 || true)"
    case "$answer" in
      *"created bucket $S3_BUCKET"*) return 0 ;;
    esac
    sleep 1
  done
  podman logs --tail 20 chat-it-s3 >&2
  return 1
}

api_binary() {
  (cd "$ROOT/api" && DATABASE_URL="$DATABASE_URL_IT" cargo build --bin api >&2)
  local target
  target="$(cd "$ROOT/api" && cargo metadata --no-deps --format-version 1 | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
  printf '%s/debug/api\n' "$target"
}

start_api() {
  local binary
  binary="${CHAT_IT_API_BIN:-$(api_binary)}"
  start_chat_api "$binary"
  start_full_api "$binary"
}

start_chat_api() {
  local binary="$1"
  env API_ROLE=rt PORT="$PORT" DATABASE_URL="$DATABASE_URL_IT" PG_POOL_MAX=8 \
    REDIS_URL="redis://127.0.0.1:$REDIS_PORT" REDIS_RT_URL="redis://127.0.0.1:$REDIS_RT_PORT" \
    CHAT_ENABLED=true CHAT_ACCESS=all CHAT_PUBLIC=on CHAT_KEYS_DIR="$STATE/keys" \
    CHAT_MEDIA_UPLOADS=true CHAT_STORES=1 CHAT_STORE_WRITE=1 CHAT_STORE_1_BUCKET="$S3_BUCKET" \
    CHAT_STORE_1_ENDPOINT="http://127.0.0.1:$S3_PORT" CHAT_STORE_1_PRESIGN="http://127.0.0.1:$S3_PORT" \
    CHAT_STORE_1_ACCESS_KEY="$S3_ACCESS_KEY" CHAT_STORE_1_CAP_GB=1 \
    CHAT_MOD_AI_ENABLED=true CHAT_MOD_AI_URL="http://127.0.0.1:$AI_PORT" CHAT_MOD_AI_MODEL=fake-judge \
    RUST_LOG="${CHAT_IT_RUST_LOG:-info,api=debug,sqlx=warn}" \
    "$binary" > "$STATE/chat-api.log" 2>&1 &
  echo $! > "$STATE/chat-api.pid"
  wait_healthy "$PORT" chat-api
}

start_full_api() {
  local binary="$1"
  env PORT="$FULL_PORT" DATABASE_URL="$DATABASE_URL_IT" PG_POOL_MAX=8 \
    REDIS_URL="redis://127.0.0.1:$REDIS_PORT" \
    STREAM_TICKET_KEY="$(random_hex 32 | xxd -r -p | base64)" \
    AUTH_X=on AUTH_KEYS_DIR="$STATE/keys" \
    RUST_LOG="${CHAT_IT_RUST_LOG:-info,api=debug,sqlx=warn}" \
    "$binary" > "$STATE/full-api.log" 2>&1 &
  echo $! > "$STATE/full-api.pid"
  wait_healthy "$FULL_PORT" full-api
}

wait_healthy() {
  local port="$1" name="$2"
  for _ in $(seq 1 60); do
    curl -fsS "http://127.0.0.1:$port/health" > /dev/null 2>&1 && return 0
    kill -0 "$(cat "$STATE/$name.pid")" 2> /dev/null || break
    sleep 1
  done
  tail -20 "$STATE/$name.log" >&2
  return 1
}

stop_api() {
  local name
  for name in chat-api full-api; do
    [ -f "$STATE/$name.pid" ] || continue
    kill "$(cat "$STATE/$name.pid")" 2> /dev/null || true
    rm -f "$STATE/$name.pid"
  done
}

print_env() {
  cat << EOF
export CHAT_IT_ORIGIN=http://127.0.0.1:$FRONT_PORT
export CHAT_IT_RELAY=http://127.0.0.1:$RELAY_PORT
export CHAT_IT_PSQL="podman exec -i $PG_CONTAINER psql -U $PG_USER -d $DB"
export CHAT_IT_REDIS_RT="podman exec -i chat-it-redis-rt redis-cli"
export CHAT_IT_ADMIN_TOKEN_FILE=$STATE/keys/chat-admin-token
export CHAT_IT_LOG=$STATE/chat-api.log
export SC_CHAT_DEV_OCK=1:$TEST_OCK_PUB
EOF
}

case "${1:-}" in
  up)
    mkdir -p "$STATE"
    stop_api
    make_keys
    migrate
    start_containers
    start_api
    print_env > "$STATE/env"
    print_env
    ;;
  restart-api)
    stop_api
    start_api
    ;;
  env)
    print_env
    ;;
  down)
    stop_api
    podman rm -f -v "${CONTAINERS[@]}" > /dev/null 2>&1 || true
    psql_in -d postgres -c "DROP DATABASE IF EXISTS $DB WITH (FORCE)" < /dev/null || true
    rm -rf "$STATE"
    ;;
  *)
    echo "usage: scripts/chat-it.sh up|restart-api|env|down" >&2
    exit 2
    ;;
esac
