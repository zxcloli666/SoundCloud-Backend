#!/bin/sh
set -eu

start() {
    case $1 in
    postgres)
        docker run -d --name postgres -p 5432:5432 --tmpfs /var/lib/postgresql/data \
            -e POSTGRES_USER=scd -e POSTGRES_PASSWORD=dev -e POSTGRES_DB=scd \
            mirror.gcr.io/library/postgres:17-alpine \
            -c fsync=off -c full_page_writes=off -c max_connections=500 -c autovacuum=off
        ;;
    redis)
        docker run -d --name redis -p 6379:6379 mirror.gcr.io/library/redis:7-alpine
        ;;
    qdrant)
        docker run -d --name qdrant -p 6333:6333 -p 6334:6334 mirror.gcr.io/qdrant/qdrant:v1.19.0
        ;;
    esac
}

schema() {
    timeout 120 sh -c 'until docker exec postgres pg_isready -h 127.0.0.1 -U scd -q; do sleep 1; done'
    docker exec postgres mkdir -p /schema
    docker cp api/migrations postgres:/schema/core
    docker cp api/migrations-ops postgres:/schema/ops
    docker exec postgres sh -c 'for file in /schema/core/*.sql /schema/ops/*.sql; do out=$(psql -U scd -d scd -v ON_ERROR_STOP=1 -q -f "$file" 2>&1) || { echo "$file: $out"; exit 1; }; done'
    echo "schema: $(ls api/migrations/*.sql api/migrations-ops/*.sql | wc -l | tr -d ' ') migrations applied"
}

up() {
    for service in "$@"; do start "$service" > /dev/null; done
    echo "started: $*"
    case " $* " in
    *" postgres "*) schema ;;
    esac
    case " $* " in
    *" qdrant "*) timeout 120 sh -c 'until curl -sf http://127.0.0.1:6333/readyz > /dev/null; do sleep 1; done' ;;
    esac
}

log=$RUNNER_TEMP/services.log

case ${1:-} in
run)
    shift
    up "$@"
    ;;
up)
    shift
    rm -f "$RUNNER_TEMP/services.done" "$RUNNER_TEMP/services.failed"
    (if sh "$0" run "$@" > "$log" 2>&1; then touch "$RUNNER_TEMP/services.done"; else touch "$RUNNER_TEMP/services.failed"; fi) < /dev/null > /dev/null 2>&1 &
    ;;
wait)
    until [ -e "$RUNNER_TEMP/services.done" ]; do
        if [ -e "$RUNNER_TEMP/services.failed" ]; then
            cat "$log"
            exit 1
        fi
        sleep 1
    done
    cat "$log"
    ;;
*)
    echo "usage: ci-db.sh up <service>... | wait" >&2
    exit 2
    ;;
esac
