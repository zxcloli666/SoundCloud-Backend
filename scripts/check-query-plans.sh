#!/usr/bin/env bash
set -uo pipefail

if [ -n "${PSQL:-}" ]; then
  read -r -a PSQL_CMD <<< "$PSQL"
else
  : "${DATABASE_URL:?DATABASE_URL required, or set PSQL='podman exec -i <container> psql -U <user> -d <db>'}"
  PSQL_CMD=(psql "$DATABASE_URL")
fi
PSQL_CMD+=(-tAqX)

QDIR="${QUERIES_DIR:-api/queries}"
BIG="${BIG_TABLES:-tracks track_artists user_events albums album_tracks sc_track_counters artists wanted_tracks user_likes_tracks playlist_track_projection playlist_membership_operations playlist_remote_snapshot_tracks listening_history lyrics_cache lyrics_lookup_state}"

cd "$(git rev-parse --show-toplevel)"

if ! "${PSQL_CMD[@]}" -c 'SELECT 1' > /dev/null 2>&1 < /dev/null; then
  echo "не удалось выполнить запрос: проверь DATABASE_URL или PSQL" >&2
  exit 2
fi

total=$(find "$QDIR" -name '*.sql' | wc -l)
if [ "$total" = 0 ]; then
  echo "в $QDIR нет ни одного .sql" >&2
  exit 2
fi

fail=0; checked=0; skipped=0
while IFS= read -r f; do
  sql=$(cat "$f")
  plan=$("${PSQL_CMD[@]}" -c "EXPLAIN (GENERIC_PLAN, FORMAT JSON) $sql" 2>/dev/null < /dev/null)
  if [ -z "$plan" ]; then
    skipped=$((skipped + 1))
    continue
  fi
  checked=$((checked + 1))
  hits=$(printf '%s' "$plan" | jq -r '[.. | objects | select(."Node Type"=="Seq Scan") | ."Relation Name"] | unique[]?' 2>/dev/null)
  for t in $hits; do
    for b in $BIG; do
      [ "$t" = "$b" ] && { printf '  ⚠ Seq Scan on %-20s %s\n' "$t" "${f#"$QDIR"/}"; fail=1; }
    done
  done
done < <(find "$QDIR" -name '*.sql' | sort)

echo "проверено $checked из $total .sql, пропущено $skipped"

if [ "$checked" = 0 ]; then
  echo "ни один запрос не разобран — манифест пуст и ничего не доказывает" >&2
  exit 2
fi

if [ "$skipped" -gt $((total / 2)) ]; then
  echo "пропущено больше половины запросов: схема в базе не та, что ждут запросы" >&2
  exit 2
fi

if [ "$fail" = 1 ]; then
  echo "↑ потенциально медленные планы (нет индекса / запрос не подхватывает существующий). Сверь EXPLAIN (ANALYZE, BUFFERS) на проде."
  [ "${FAIL_ON_SEQSCAN:-0}" = 1 ] && exit 1
fi
exit 0
