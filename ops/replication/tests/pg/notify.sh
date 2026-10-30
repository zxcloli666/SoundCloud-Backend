#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repl="$(cd "$here/../.." && pwd)"
. "$here/lib.sh"

template="${TEMPLATE_DB:-build}"
host="${PG_HOST:-127.0.0.1}"
port="${PG_PORT:-5432}"
client="${CLIENT_URL_HOST:-$host:$port}"

cleanup() {
  for n in core main; do
    for s in $(val postgres "SELECT s.subname FROM pg_catalog.pg_subscription s JOIN pg_catalog.pg_database d ON d.oid = s.subdbid WHERE d.datname = 'scd_n_$n'"); do
      sql -d "scd_n_$n" -c "ALTER SUBSCRIPTION $s DISABLE" -c "ALTER SUBSCRIPTION $s SET (slot_name = NONE)" -c "DROP SUBSCRIPTION $s"
    done
  done
  for s in $(val postgres "SELECT slot_name FROM pg_catalog.pg_replication_slots WHERE slot_name LIKE 'scd_n_%'"); do
    val postgres "SELECT pg_catalog.pg_drop_replication_slot('$s')" > /dev/null
  done
  for n in core main; do
    sql -d postgres -c "DROP DATABASE IF EXISTS scd_n_$n WITH (FORCE)"
  done
}
trap 'cleanup > /dev/null 2>&1 || true' EXIT
cleanup

val postgres "SELECT count(*) FROM pg_catalog.pg_roles WHERE rolname = 'scd_repl'" | grep -q 1 || sql -d postgres -c "CREATE ROLE scd_repl NOLOGIN"
for n in core main; do
  fresh "scd_n_$n" "$template"
  role=api; [ "$n" = core ] && role=core
  for f in tables triggers row_hash row_json registry purge_children publications; do
    sql -d "scd_n_$n" -c "SET search_path = ''" -c "SET client_min_messages = warning" -f - < "$repl/generated/$f.sql"
  done
  val "scd_n_$n" "SELECT count(*) FROM public.scd_configure('$n', '$role', 'split')" > /dev/null
  val "scd_n_$n" "SELECT count(*) FROM public.scd_grant_repl()" > /dev/null
  val "scd_n_$n" "SELECT count(*) FROM public.scd_sync_publications()" > /dev/null
done

subscribe() {
  val "scd_n_$2" "SELECT pg_catalog.pg_create_logical_replication_slot('scd_n_$1_from_$2', 'pgoutput')" > /dev/null
  sql -d "scd_n_$1" -c "CREATE SUBSCRIPTION $1_from_$2 CONNECTION 'host=$host port=$port dbname=scd_n_$2 user=postgres password=pw' PUBLICATION $3 WITH ($4, create_slot = false, slot_name = 'scd_n_$1_from_$2', copy_data = false)"
}
subscribe core main "scd_api_pub, scd_cmd_pub" "origin = none"
subscribe main core scd_core_pub "origin = any, disable_on_error = false, streaming = parallel"
for n in core main; do
  wait_for "scd_n_$n" "SELECT bool_and(pid IS NOT NULL) FROM pg_catalog.pg_stat_subscription WHERE relid IS NULL AND leader_pid IS NULL" "apply worker up on $n"
done

SCD_API_URL="postgres://postgres:pw@$client/scd_n_main" \
SCD_CORE_URL="postgres://postgres:pw@$client/scd_n_core" \
  "$@"
