keep_install="$(ls "$root"/api/migrations/*_scd_install.sql | sort | tail -1)"
keep_triggers() {
  only keep "SELECT md5(string_agg(line, E'\n' ORDER BY line)) FROM (
    SELECT t.tgrelid::regclass::text || ' ' || t.tgname || ' ' || t.tgenabled::text || ' ' || pg_catalog.pg_get_triggerdef(t.oid) AS line
    FROM pg_catalog.pg_trigger AS t
    WHERE NOT t.tgisinternal
  ) AS triggers"
}

say "scd_install: a re-run takes no lock on tables whose triggers did not change"
fresh scd_b1_keep scd_b1_tpl
kept="$(sed -n "s/^SELECT public\.scd_keep_trigger('\([^']*\)', .*/\1/p" "$keep_install" | sort -u)"
made="$(sed -n 's/^CREATE OR REPLACE TRIGGER [^ ]* .* ON \([^ ]*\) FOR EACH .*/\1/p' "$keep_install" | sort -u)"
locked="$(comm -23 <(printf '%s\n' "$kept") <(printf '%s\n' "$made") | paste -sd, -)"
count="$(printf '%s' "$locked" | tr ',' '\n' | grep -c . || true)"
[ "$count" -gt 100 ] || { echo "FAIL $(basename "$keep_install") keeps triggers on only $count tables"; exit 1; }
sql -d scd_b1_keep -c "SET application_name = 'scd_keep'" -c "BEGIN" -c "LOCK TABLE $locked IN EXCLUSIVE MODE" \
  -c "SELECT pg_sleep(600)" -c "COMMIT" > "$tmp/keep.out" 2>&1 &
keep_holder=$!
wait_for scd_b1_keep "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = 'scd_keep' AND wait_event = 'PgSleep')" "another session holds EXCLUSIVE on the $count tables with kept triggers"
before="$(keep_triggers)"
if ! sql -d scd_b1_keep -c "SET client_min_messages = warning" -f - < "$keep_install" > "$tmp/keep.log" 2>&1; then
  echo "FAIL the re-run of $(basename "$keep_install") waited on a lock: $(grep -m1 ERROR "$tmp/keep.log")"
  exit 1
fi
want "the re-run of $(basename "$keep_install") ran under those locks and changed no trigger" "$(keep_triggers)" "$before"
val scd_b1_keep "SELECT count(*) FROM (SELECT pg_catalog.pg_terminate_backend(pid) FROM pg_catalog.pg_stat_activity WHERE application_name = 'scd_keep') AS x" > /dev/null
wait "$keep_holder" || true

say "scd_install: a re-run makes a missing trigger and restores a replica trigger's state"
plain="$(sed -n "s/^SELECT public\.scd_keep_trigger('\([^']*\)', '\([^']*\)', false, .*/\1 \2/p" "$keep_install" | head -1)"
replica="$(sed -n "s/^SELECT public\.scd_keep_trigger('\([^']*\)', '\([^']*\)', true, .*/\1 \2/p" "$keep_install" | head -1)"
sql -d scd_b1_keep -c "DROP TRIGGER ${plain#* } ON ${plain% *}" -c "ALTER TABLE ${replica% *} ENABLE TRIGGER ${replica#* }" > /dev/null
want "${plain#* } is gone and ${replica#* } fires on every write" "$(only keep "SELECT count(*) FILTER (WHERE tgname = '${plain#* }') || ' ' || string_agg(tgenabled::text, '') FILTER (WHERE tgname = '${replica#* }') FROM pg_catalog.pg_trigger WHERE tgrelid IN ('${plain% *}'::regclass, '${replica% *}'::regclass)")" "0 O"
sql -d scd_b1_keep -c "SET client_min_messages = warning" -f - < "$keep_install" > /dev/null
want "the re-run brings both back as the install made them" "$(keep_triggers)" "$before"
sql -d postgres -c "DROP DATABASE scd_b1_keep WITH (FORCE)" > /dev/null
