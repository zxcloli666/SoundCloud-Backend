psql_cmd="${PSQL:-psql}"
socket="${PG_SOCKET:-/var/run/postgresql}"
nodes="core main star"

sql() { $psql_cmd -X -q -v ON_ERROR_STOP=1 -v FETCH_COUNT=5000 "$@"; }
val() { $psql_cmd -X -q -v ON_ERROR_STOP=1 -v FETCH_COUNT=5000 -At -d "$1" -c "$2"; }
say() { printf '== %s\n' "$*"; }

teardown() {
  for n in $nodes; do
    for s in $(val postgres "SELECT s.subname FROM pg_catalog.pg_subscription s JOIN pg_catalog.pg_database d ON d.oid = s.subdbid WHERE d.datname = 'scd_b1_$n'"); do
      sql -d "scd_b1_$n" -c "ALTER SUBSCRIPTION $s DISABLE" -c "ALTER SUBSCRIPTION $s SET (slot_name = NONE)" -c "DROP SUBSCRIPTION $s"
    done
  done
  for s in $(val postgres "SELECT slot_name FROM pg_catalog.pg_replication_slots WHERE slot_name LIKE 'scd_b1_%'"); do
    val postgres "SELECT pg_catalog.pg_drop_replication_slot('$s')" > /dev/null
  done
  for d in scd_b1_core scd_b1_main scd_b1_star scd_b1_smoke scd_b1_exercise scd_b1_race scd_b1_tpl scd_b1_ops; do
    sql -d postgres -c "DROP DATABASE IF EXISTS $d WITH (FORCE)"
  done
}

fresh() {
  sql -d postgres -c "SET client_min_messages = warning" -c "DROP DATABASE IF EXISTS $1 WITH (FORCE)" -c "CREATE DATABASE $1 ${2:+TEMPLATE $2}"
}

weak_grant() {
  local out
  out="$(sql -At -d "$1" -c "SET client_min_messages = error" -f - <<'SQL'
BEGIN;
REVOKE SET ON PARAMETER session_replication_role FROM scd_repl;
CREATE ROLE scd_b2_owner NOLOGIN;
GRANT USAGE ON SCHEMA public TO scd_b2_owner WITH GRANT OPTION;
GRANT ALL ON ALL TABLES IN SCHEMA public TO scd_b2_owner WITH GRANT OPTION;
GRANT ALL ON ALL SEQUENCES IN SCHEMA public TO scd_b2_owner WITH GRANT OPTION;
SET LOCAL ROLE scd_b2_owner;
SELECT string_agg(x, '; ') FROM public.scd_grant_repl() x;
RESET ROLE;
SELECT string_agg(x, '; ') FROM public.scd_repl_problems() x;
ROLLBACK;
SQL
)"
  case "$out" in
    *"skip set on parameter"*"skip scd_dispatch owner"*"lacks SET on parameter"*"scd_dispatch is owned by"*) ;;
    *) echo "FAIL scd_grant_repl as a non-superuser owner: $out"; exit 1 ;;
  esac
  echo "ok scd_grant_repl without superuser grants the tables, skips the parameter and the dispatch owner, scd_repl_problems keeps both"
}

install() {
  for f in tables triggers row_hash row_json registry purge_children publications; do
    sql -d "$1" -c "SET search_path = ''" -c "SET client_min_messages = warning" -f - < "$repl/generated/$f.sql"
  done
}

wait_for() {
  local i
  for i in $(seq 1 120); do
    if [ "$(val "$1" "$2")" = t ]; then
      echo "ok $3"
      return 0
    fi
    sleep 0.5
  done
  echo "FAIL $3 (on $1: $2)"
  exit 1
}

mark() {
  sql -d "scd_b1_$1" -c "INSERT INTO public.user_events (id, sc_user_id, sc_track_id, event_type, weight) VALUES ('00000000-0000-0000-0000-00000000$2', 'mark', 'mark', 'mark', 0)"
}

same_on_all() {
  local a b c
  a="$(val scd_b1_core "$1")"; b="$(val scd_b1_main "$1")"; c="$(val scd_b1_star "$1")"
  if [ -z "$a" ] || [ "$a" != "$b" ] || [ "$a" != "$c" ]; then
    echo "FAIL $2: core=$a main=$b star=$c"
    exit 1
  fi
  echo "ok $2"
}

rows() {
  val "scd_b1_$1" "SELECT (SELECT jsonb_object_agg(e.key, e.value) FROM jsonb_each(to_jsonb(t)) e WHERE l.cols IS NULL OR e.key = ANY (l.cols))::text
    FROM public.$2 t LEFT JOIN (SELECT cols FROM public.scd_publication_lists() WHERE pubname = 'scd_core_pub' AND tbl = '$2') l ON true
    WHERE $3 ORDER BY 1"
}

pairs() {
  local n a b d out=""
  for n in $nodes; do rows "$n" "$1" "${2:-true}" > "$tmp/rows.$n"; done
  for p in core:main core:star main:star; do
    a="${p%%:*}"; b="${p##*:}"
    d="$(diff "$tmp/rows.$a" "$tmp/rows.$b" | grep -c '^[<>]' || true)"
    out="$out $a/$b=$d"
    [ "$d" = 0 ] || { echo "FAIL $3: $1 differing lines$out"; diff "$tmp/rows.$a" "$tmp/rows.$b" | head -6; exit 1; }
  done
  echo "ok $3 ($1 rows $(wc -l < "$tmp/rows.core" | tr -d ' '), differing lines$out)"
}

pause() {
  sql -d "scd_b1_$1" -c "ALTER SUBSCRIPTION $2 DISABLE"
  wait_for "scd_b1_$1" "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = '$2' AND pid IS NOT NULL)" "$1 stops $2"
}

resume() {
  sql -d "scd_b1_$1" -c "ALTER SUBSCRIPTION $2 ENABLE"
}

settle() {
  mark main "$1"
  mark star "$2"
  wait_for scd_b1_core "SELECT count(*) = 2 FROM public.user_events WHERE id IN ('00000000-0000-0000-0000-00000000$1', '00000000-0000-0000-0000-00000000$2')" "core consumed main and star"
  mark core "$3"
  wait_for scd_b1_main "SELECT count(*) = 3 FROM public.user_events WHERE id IN ('00000000-0000-0000-0000-00000000$1', '00000000-0000-0000-0000-00000000$2', '00000000-0000-0000-0000-00000000$3')" "main consumed core"
  wait_for scd_b1_star "SELECT count(*) = 3 FROM public.user_events WHERE id IN ('00000000-0000-0000-0000-00000000$1', '00000000-0000-0000-0000-00000000$2', '00000000-0000-0000-0000-00000000$3')" "star consumed core"
}
