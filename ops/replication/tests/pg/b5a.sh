reads_at() { only "$1" "SELECT coalesce((SELECT to_char(last_read_at AT TIME ZONE 'UTC', 'HH24:MI') FROM public.entity_reads WHERE entity_urn = '$2'), 'none')"; }
parked() { only "$1" "SELECT sync_status || ' ' || reconcile_failure_streak FROM public.playlist_membership_state WHERE playlist_urn = '$2'"; }
park() {
  sql -d "scd_b1_$1" -c "UPDATE public.playlist_membership_state SET sync_status = 'auth_required', next_reconcile_at = now() + interval '17 hours', reconcile_failure_streak = 9 WHERE playlist_urn = '$2'" > /dev/null
}

say "B5a: a read stamp goes from an api node to core only, core keeps the newest, the other api node never sees it (T7)"
t1=soundcloud:tracks:b5a1
sql -d scd_b1_main -c "INSERT INTO public.entity_reads (entity_urn, last_read_at) VALUES ('$t1', '2026-10-31 08:00:00+00')" > /dev/null
sql -d scd_b1_star -c "INSERT INTO public.entity_reads (entity_urn, last_read_at) VALUES ('$t1', '2026-10-31 07:00:00+00')" > /dev/null
calm
want "core keeps main's newer stamp over star's older insert" "$(reads_at core "$t1")" 08:00
want "star keeps its own stamp" "$(reads_at star "$t1")" 07:00
sql -d scd_b1_star -c "UPDATE public.entity_reads SET last_read_at = '2026-10-31 09:00:00+00' WHERE entity_urn = '$t1'" > /dev/null
calm
want "core takes star's later stamp" "$(reads_at core "$t1")" 09:00
want "main never receives star's stamp" "$(reads_at main "$t1")" 08:00
sql -d scd_b1_main -c "UPDATE public.entity_reads SET last_read_at = '2026-10-31 08:30:00+00' WHERE entity_urn = '$t1'" > /dev/null
calm
want "an older update from main does not move core back" "$(reads_at core "$t1")" 09:00
want "core never writes its max back to an api node" "$(reads_at main "$t1")/$(reads_at star "$t1")" 08:30/09:00

say "B5a: a read of a public playlist parked without its owner wakes it on core, writes the state row before the playlist, and the wake reaches both api nodes"
p1=soundcloud:playlists:b5a1
sql -d scd_b1_core > /dev/null <<SQL
CREATE SCHEMA b5a;
CREATE TABLE b5a.writes (n bigserial PRIMARY KEY, tbl text NOT NULL);
GRANT USAGE ON SCHEMA b5a TO scd_repl;
GRANT SELECT, INSERT ON b5a.writes TO scd_repl;
GRANT USAGE ON SEQUENCE b5a.writes_n_seq TO scd_repl;
CREATE FUNCTION b5a.wrote() RETURNS trigger LANGUAGE plpgsql AS \$f\$
BEGIN
    INSERT INTO b5a.writes (tbl) VALUES (TG_TABLE_NAME);
    RETURN NULL;
END
\$f\$;
CREATE TRIGGER b5a_state AFTER UPDATE ON public.playlist_membership_state FOR EACH ROW WHEN (NEW.playlist_urn = '$p1') EXECUTE FUNCTION b5a.wrote();
CREATE TRIGGER b5a_playlist AFTER UPDATE ON public.playlists FOR EACH ROW WHEN (NEW.urn = '$p1') EXECUTE FUNCTION b5a.wrote();
ALTER TABLE public.playlist_membership_state ENABLE ALWAYS TRIGGER b5a_state;
ALTER TABLE public.playlists ENABLE ALWAYS TRIGGER b5a_playlist;
INSERT INTO public.playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, sharing)
VALUES ('b5a1', '$p1', 'Stranger mix', 'stranger mix', '17', 'public');
INSERT INTO public.playlist_membership_state (playlist_urn) VALUES ('$p1');
SQL
park core "$p1"
calm
want "main has the parked playlist" "$(parked main "$p1")" "auth_required 9"
sql -d scd_b1_core -c "TRUNCATE b5a.writes" > /dev/null
pause main main_from_core
sql -d scd_b1_star -c "INSERT INTO public.entity_reads (entity_urn, last_read_at) VALUES ('$p1', now())" > /dev/null
wait_for scd_b1_core "SELECT sync_status = 'retry_wait' AND reconcile_failure_streak = 0 AND next_reconcile_at < now() + interval '1 minute' FROM public.playlist_membership_state WHERE playlist_urn = '$p1'" "core wakes the playlist star read"
want "core writes the state row before the playlist" "$(only core "SELECT string_agg(tbl, ' ' ORDER BY n) FROM b5a.writes")" "playlist_membership_state playlists"
want "core stamps the playlist with star's read" "$(only core "SELECT p.last_read_at = r.last_read_at FROM public.playlists p JOIN public.entity_reads r ON r.entity_urn = p.urn WHERE p.urn = '$p1'")" t
wait_for scd_b1_star "SELECT sync_status = 'retry_wait' FROM public.playlist_membership_state WHERE playlist_urn = '$p1'" "star receives the wake from core"
want "star receives core's stamp, a core column" "$(only star "SELECT last_read_at FROM public.playlists WHERE urn = '$p1'")" "$(only core "SELECT last_read_at FROM public.playlists WHERE urn = '$p1'")"
want "main, cut off from core, still has it parked: an api node never wakes locally" "$(parked main "$p1")" "auth_required 9"
resume main main_from_core
wait_for scd_b1_main "SELECT sync_status = 'retry_wait' FROM public.playlist_membership_state WHERE playlist_urn = '$p1'" "main receives the wake once it reads core again"
want "and core's stamp" "$(only main "SELECT last_read_at FROM public.playlists WHERE urn = '$p1'")" "$(only core "SELECT last_read_at FROM public.playlists WHERE urn = '$p1'")"
stamp="$(only main "SELECT last_read_at FROM public.playlists WHERE urn = '$p1'")"
sql -d scd_b1_main -c "UPDATE public.playlists SET last_read_at = now() - interval '3 days' WHERE urn = '$p1'" > /dev/null
want "a local write on an api node keeps core's stamp" "$(only main "SELECT last_read_at FROM public.playlists WHERE urn = '$p1'")" "$stamp"

say "B5a: a second read inside 6 hours leaves a parked playlist parked, a read after 6 hours wakes it again"
park core "$p1"
sql -d scd_b1_main -c "INSERT INTO public.entity_reads (entity_urn, last_read_at) VALUES ('$p1', now())" > /dev/null
calm
want "a read 0 h after the wake keeps it parked on core" "$(parked core "$p1")" "auth_required 9"
want "and on the api nodes" "$(parked main "$p1")/$(parked star "$p1")" "auth_required 9/auth_required 9"
sql -d scd_b1_core -c "UPDATE public.playlists SET last_read_at = now() - interval '7 hours' WHERE urn = '$p1'" -c "TRUNCATE b5a.writes" > /dev/null
sql -d scd_b1_main -c "UPDATE public.entity_reads SET last_read_at = now() WHERE entity_urn = '$p1'" > /dev/null
calm
want "a read 7 h after the last stamp wakes it again" "$(parked core "$p1")" "retry_wait 0"
want "the update path writes the state row first too" "$(only core "SELECT string_agg(tbl, ' ' ORDER BY n) FROM b5a.writes")" "playlist_membership_state playlists"
want "both api nodes have the second wake" "$(parked main "$p1")/$(parked star "$p1")" "retry_wait 0/retry_wait 0"

say "B5a: a private playlist and a playing state are never woken"
sql -d scd_b1_core -c "UPDATE public.playlists SET sharing = 'private', last_read_at = now() - interval '7 hours' WHERE urn = '$p1'" > /dev/null
park core "$p1"
sql -d scd_b1_star -c "UPDATE public.entity_reads SET last_read_at = now() + interval '1 second' WHERE entity_urn = '$p1'" > /dev/null
calm
want "a private playlist stays parked" "$(parked core "$p1")" "auth_required 9"
sql -d scd_b1_core -c "UPDATE public.playlists SET sharing = 'public', last_read_at = now() - interval '7 hours' WHERE urn = '$p1'" -c "UPDATE public.playlist_membership_state SET sync_status = 'clean', reconcile_failure_streak = 2 WHERE playlist_urn = '$p1'" > /dev/null
sql -d scd_b1_main -c "UPDATE public.entity_reads SET last_read_at = now() + interval '2 seconds' WHERE entity_urn = '$p1'" > /dev/null
calm
want "a state that is not auth_required is left alone" "$(parked core "$p1")" "clean 2"
sql -d scd_b1_core -c "DROP TRIGGER b5a_state ON public.playlist_membership_state" -c "DROP TRIGGER b5a_playlist ON public.playlists" -c "DROP SCHEMA b5a CASCADE" > /dev/null

touch_sql="PREPARE touch(text) AS $(cat "$root/api/queries/playlists/touch_last_read.sql")"
recent_sql="PREPARE recent(text) AS $(cat "$root/jobs/queries/playlist_observe/is_recently_viewed_public.sql")"
view() { sql -d "scd_b1_$1" -c "$touch_sql" -c "EXECUTE touch('$2')" > /dev/null; }
recent() { sql -At -d "scd_b1_$1" -c "$recent_sql" -c "EXECUTE recent('$2')"; }

say "B5a: an api node sends a read of a playlist whose core stamp is over 6 h old at once, not after its 1 h throttle, and core's new stamp comes back"
p3=soundcloud:playlists:b5a3
sql -d scd_b1_core -c "INSERT INTO public.playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, sharing, last_read_at)
  VALUES ('b5a3', '$p3', 'Due mix', 'due mix', '17', 'public', now() - interval '6 hours 30 minutes')" > /dev/null
sql -d scd_b1_main -c "INSERT INTO public.entity_reads (entity_urn, last_read_at) VALUES ('$p3', now() - interval '40 minutes')" > /dev/null
calm
want "main has core's 6.5 h old stamp" "$(only main "SELECT last_read_at < now() - interval '6 hours' FROM public.playlists WHERE urn = '$p3'")" t
want "a read 40 minutes old does not move it" "$(only core "SELECT last_read_at < now() - interval '6 hours' FROM public.playlists WHERE urn = '$p3'")" t
view main "$p3"
want "main writes the read at once" "$(only main "SELECT last_read_at > now() - interval '1 minute' FROM public.entity_reads WHERE entity_urn = '$p3'")" t
wait_for scd_b1_main "SELECT last_read_at > now() - interval '1 minute' FROM public.playlists WHERE urn = '$p3'" "core stamps the playlist and the stamp comes back to main"
seen="$(only main "SELECT xmin || ' ' || last_read_at FROM public.entity_reads WHERE entity_urn = '$p3'")"
view main "$p3"
want "a second read with a fresh stamp writes nothing" "$(only main "SELECT xmin || ' ' || last_read_at FROM public.entity_reads WHERE entity_urn = '$p3'")" "$seen"
sql -d scd_b1_main -c "UPDATE public.entity_reads SET last_read_at = now() - interval '2 hours' WHERE entity_urn = '$p3'" > /dev/null
view main "$p3"
want "a read after the 1 h throttle writes again" "$(only main "SELECT last_read_at > now() - interval '1 minute' FROM public.entity_reads WHERE entity_urn = '$p3'")" t
want "jobs on core sees the view" "$(recent core "$p3")" t

say "B5a S2: core seeded with the max of main's and star's K rows before the api subscriptions start merges later inserts and takes later updates"
k1=soundcloud:tracks:b5as1
k2=soundcloud:tracks:b5as2
k3=soundcloud:tracks:b5as3
k4=soundcloud:tracks:b5as4
errs() { only core "SELECT coalesce(sum(apply_error_count), 0) FROM pg_catalog.pg_stat_subscription_stats WHERE subname IN ('core_from_main', 'core_from_star')"; }
plays() { only "$1" "SELECT coalesce((SELECT play_count || ' ' || to_char(fetched_at AT TIME ZONE 'UTC', 'HH24:MI') FROM public.sc_track_counters WHERE sc_track_id = '$2'), 'none')"; }
before() {
  sql -d "scd_b1_$1" -c "SELECT pg_catalog.pg_replication_origin_create('b5a_before')" \
    -c "SELECT pg_catalog.pg_replication_origin_session_setup('b5a_before')" -c "$2" \
    -c "SELECT pg_catalog.pg_replication_origin_session_reset()" -c "SELECT pg_catalog.pg_replication_origin_drop('b5a_before')" > /dev/null
}
e0="$(errs)"
before main "INSERT INTO public.entity_reads (entity_urn, last_read_at) VALUES ('$k1', '2026-10-31 08:00+00'), ('$k2', '2026-10-31 08:00+00'), ('$k3', '2026-10-31 08:00+00'), ('$k4', '2026-10-31 08:00+00');
  INSERT INTO public.sc_track_counters (sc_track_id, play_count, fetched_at) VALUES ('b5as1', 10, '2026-10-31 08:00+00'), ('b5as2', 5, '2026-10-31 08:00+00'), ('b5as4', 7, '2026-10-31 08:00+00')"
before star "INSERT INTO public.entity_reads (entity_urn, last_read_at) VALUES ('$k1', '2026-10-31 09:30+00');
  INSERT INTO public.sc_track_counters (sc_track_id, play_count, fetched_at) VALUES ('b5as1', 12, '2026-10-31 09:00+00')"
calm
want "rows written before the subscriptions are not on core" "$(only core "SELECT count(*) FROM public.entity_reads WHERE entity_urn LIKE 'soundcloud:tracks:b5as%'")/$(only core "SELECT count(*) FROM public.sc_track_counters WHERE sc_track_id LIKE 'b5as%'")" 0/0
for n in main star; do
  sql -At -d "scd_b1_$n" -c "COPY (SELECT entity_urn, last_read_at FROM public.entity_reads WHERE entity_urn IN ('$k1', '$k2', '$k3')) TO STDOUT" > "$tmp/reads.$n"
  sql -At -d "scd_b1_$n" -c "COPY (SELECT sc_track_id, play_count, likes_count, reposts_count, comment_count, fetched_at FROM public.sc_track_counters WHERE sc_track_id IN ('b5as1', 'b5as2')) TO STDOUT" > "$tmp/counters.$n"
done
sql -d scd_b1_core > /dev/null <<SQL
CREATE TEMP TABLE seed_reads (entity_urn text, last_read_at timestamptz);
CREATE TEMP TABLE seed_counters (sc_track_id text, play_count bigint, likes_count bigint, reposts_count bigint, comment_count bigint, fetched_at timestamptz);
COPY seed_reads FROM STDIN;
$(cat "$tmp/reads.main" "$tmp/reads.star")
\.
COPY seed_counters FROM STDIN;
$(cat "$tmp/counters.main" "$tmp/counters.star")
\.
INSERT INTO public.entity_reads AS r (entity_urn, last_read_at)
SELECT entity_urn, max(last_read_at) FROM seed_reads GROUP BY entity_urn
ON CONFLICT (entity_urn) DO UPDATE SET last_read_at = EXCLUDED.last_read_at WHERE r.last_read_at < EXCLUDED.last_read_at;
INSERT INTO public.sc_track_counters AS c (sc_track_id, play_count, likes_count, reposts_count, comment_count, fetched_at)
SELECT DISTINCT ON (sc_track_id) sc_track_id, play_count, likes_count, reposts_count, comment_count, fetched_at
FROM seed_counters ORDER BY sc_track_id, fetched_at DESC
ON CONFLICT (sc_track_id) DO UPDATE
SET play_count = EXCLUDED.play_count, likes_count = EXCLUDED.likes_count, reposts_count = EXCLUDED.reposts_count,
    comment_count = EXCLUDED.comment_count, fetched_at = EXCLUDED.fetched_at
WHERE c.fetched_at < EXCLUDED.fetched_at;
SQL
want "the seed keeps the newer of both nodes" "$(reads_at core "$k1") $(plays core b5as1)" "09:30 12 09:00"
sql -d scd_b1_main -c "UPDATE public.entity_reads SET last_read_at = '2026-10-31 10:00+00' WHERE entity_urn IN ('$k1', '$k4')" \
  -c "UPDATE public.sc_track_counters SET play_count = 15, fetched_at = '2026-10-31 10:00+00' WHERE sc_track_id IN ('b5as1', 'b5as4')" > /dev/null
calm
want "a later update from main lands on a seeded key" "$(reads_at core "$k1") $(plays core b5as1)" "10:00 15 10:00"
want "without the seed the same update is lost (the reason for the seed)" "$(reads_at core "$k4") $(plays core b5as4)" "none none"
sql -d scd_b1_star -c "UPDATE public.entity_reads SET last_read_at = '2026-10-31 09:45+00' WHERE entity_urn = '$k1'" \
  -c "UPDATE public.sc_track_counters SET play_count = 13, fetched_at = '2026-10-31 09:45+00' WHERE sc_track_id = 'b5as1'" \
  -c "INSERT INTO public.entity_reads (entity_urn, last_read_at) VALUES ('$k2', '2026-10-31 07:00+00'), ('$k3', '2026-10-31 11:00+00')" \
  -c "INSERT INTO public.sc_track_counters (sc_track_id, play_count, fetched_at) VALUES ('b5as2', 3, '2026-10-31 07:00+00')" > /dev/null
calm
want "an older update from star leaves the seeded max" "$(reads_at core "$k1") $(plays core b5as1)" "10:00 15 10:00"
want "star's insert of a key core has: an older one keeps core's value, a newer one wins" "$(reads_at core "$k2") $(reads_at core "$k3") $(plays core b5as2)" "08:00 11:00 5 08:00"
want "no apply error on core" "$(errs)" "$e0"
want "core writes none of it back" "$(reads_at main "$k3") $(reads_at star "$k4")" "08:00 none"

say "B5a legacy: an api node wakes locally from its own read, an applied change never wakes"
fresh scd_b1_legacy_main scd_b1_tpl
sql -d scd_b1_legacy_main > /dev/null <<SQL
INSERT INTO public.playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, sharing)
VALUES ('b5a1', '$p1', 'Stranger mix', 'stranger mix', '17', 'public');
INSERT INTO public.playlist_membership_state (playlist_urn, sync_status, next_reconcile_at, reconcile_failure_streak)
VALUES ('$p1', 'auth_required', now() + interval '17 hours', 9);
SET session_replication_role = replica;
INSERT INTO public.entity_reads (entity_urn, last_read_at) VALUES ('$p1', now());
SQL
want "an applied read does not wake" "$(parked legacy_main "$p1")" "auth_required 9"
sql -d scd_b1_legacy_main -c "UPDATE public.entity_reads SET last_read_at = now() + interval '1 second' WHERE entity_urn = '$p1'" > /dev/null
want "the node's own read wakes it" "$(parked legacy_main "$p1")" "retry_wait 0"
want "and stamps the playlist" "$(only legacy_main "SELECT last_read_at IS NOT NULL FROM public.playlists WHERE urn = '$p1'")" t
sql -d postgres -c "DROP DATABASE scd_b1_legacy_main WITH (FORCE)" > /dev/null

say "B5a legacy over real replication: a view on star reaches jobs on main through the playlist stamp in the read's own transaction; entity_reads stays on star"
hub_subs="core:core_from_main core:core_from_star main:main_from_core star:star_from_core"
for s in $hub_subs; do pause "${s%%:*}" "${s##*:}"; done
fresh scd_b1_legacy_main scd_b1_tpl
fresh scd_b1_legacy_star scd_b1_tpl
sql -d scd_b1_legacy_star -c "CREATE PUBLICATION b5a_legacy_pub FOR TABLE public.playlists" > /dev/null
only legacy_star "SELECT pg_catalog.pg_create_logical_replication_slot('scd_b1_legacy_main_from_star', 'pgoutput')" > /dev/null
sql -d scd_b1_legacy_main -c "CREATE SUBSCRIPTION legacy_main_from_star CONNECTION 'host=$socket dbname=scd_b1_legacy_star user=scd' PUBLICATION b5a_legacy_pub WITH (origin = none, create_slot = false, slot_name = 'scd_b1_legacy_main_from_star', copy_data = false)"
wait_for scd_b1_legacy_main "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = 'legacy_main_from_star' AND relid IS NULL AND pid IS NOT NULL)" "legacy main reads star"
p2=soundcloud:playlists:b5a2
sql -d scd_b1_legacy_star -c "INSERT INTO public.playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, sharing, last_read_at)
  VALUES ('b5a2', '$p2', 'Old mix', 'old mix', '17', 'public', now() - interval '8 days')" > /dev/null
wait_for scd_b1_legacy_main "SELECT EXISTS (SELECT 1 FROM public.playlists WHERE urn = '$p2')" "main has star's playlist"
want "jobs on main sees no view of an 8 day old stamp" "$(recent legacy_main "$p2")" f
view legacy_star "$p2"
wait_for scd_b1_legacy_main "SELECT last_read_at > now() - interval '1 minute' FROM public.playlists WHERE urn = '$p2'" "star's view reaches main"
echo "ok seen on main $(only legacy_main "SELECT round(extract(epoch FROM clock_timestamp() - last_read_at) * 1000) FROM public.playlists WHERE urn = '$p2'") ms after the read (0.5 s poll)"
want "jobs on main sees the view" "$(recent legacy_main "$p2")" t
want "star moves the stamp in the read's own transaction" "$(only legacy_star "SELECT r.xmin = p.xmin AND r.last_read_at = p.last_read_at FROM public.entity_reads r JOIN public.playlists p ON p.urn = r.entity_urn WHERE r.entity_urn = '$p2'")" t
want "entity_reads never leaves star" "$(only legacy_main "SELECT count(*) FROM public.entity_reads")" 0
sql -d scd_b1_legacy_star -c "UPDATE public.entity_reads SET last_read_at = now() - interval '30 minutes' WHERE entity_urn = '$p2'; UPDATE public.playlists SET last_read_at = now() - interval '6 hours 30 minutes' WHERE urn = '$p2'" > /dev/null
wait_for scd_b1_legacy_main "SELECT last_read_at < now() - interval '6 hours' FROM public.playlists WHERE urn = '$p2'" "main has the 6.5 h old stamp"
view legacy_star "$p2"
wait_for scd_b1_legacy_main "SELECT last_read_at > now() - interval '1 minute' FROM public.playlists WHERE urn = '$p2'" "a view 30 minutes after the last one still reaches main at once when the stamp is over 6 h old"
sql -d scd_b1_legacy_star -c "UPDATE public.entity_reads SET last_read_at = now() - interval '30 minutes' WHERE entity_urn = '$p2'; UPDATE public.playlists SET last_read_at = now() - interval '2 hours' WHERE urn = '$p2'" > /dev/null
wait_for scd_b1_legacy_main "SELECT last_read_at < now() - interval '1 hour' FROM public.playlists WHERE urn = '$p2'" "main has the 2 h old stamp"
seen="$(only legacy_star "SELECT r.xmin || ' ' || p.xmin FROM public.entity_reads r JOIN public.playlists p ON p.urn = r.entity_urn WHERE r.entity_urn = '$p2'")"
view legacy_star "$p2"
want "a view inside the hour with a fresh stamp writes nothing" "$(only legacy_star "SELECT r.xmin || ' ' || p.xmin FROM public.entity_reads r JOIN public.playlists p ON p.urn = r.entity_urn WHERE r.entity_urn = '$p2'")" "$seen"
sql -d scd_b1_legacy_star -c "UPDATE public.entity_reads SET last_read_at = now() - interval '2 hours' WHERE entity_urn = '$p2'" > /dev/null
seen="$(only legacy_star "SELECT xmin FROM public.playlists WHERE urn = '$p2'")"
view legacy_star "$p2"
want "a view after the hour moves entity_reads, not a 2 h old stamp" "$(only legacy_star "SELECT r.last_read_at > now() - interval '1 minute' AND p.xmin::text = '$seen' FROM public.entity_reads r JOIN public.playlists p ON p.urn = r.entity_urn WHERE r.entity_urn = '$p2'")" t
want "jobs on main still sees the view" "$(recent legacy_main "$p2")" t
want "no apply error on legacy main" "$(only legacy_main "SELECT apply_error_count FROM pg_catalog.pg_stat_subscription_stats WHERE subname = 'legacy_main_from_star'")" 0
sql -d scd_b1_legacy_main -c "ALTER SUBSCRIPTION legacy_main_from_star DISABLE" -c "ALTER SUBSCRIPTION legacy_main_from_star SET (slot_name = NONE)" -c "DROP SUBSCRIPTION legacy_main_from_star" > /dev/null
wait_for postgres "SELECT NOT active FROM pg_catalog.pg_replication_slots WHERE slot_name = 'scd_b1_legacy_main_from_star'" "the legacy slot is free"
val postgres "SELECT pg_catalog.pg_drop_replication_slot('scd_b1_legacy_main_from_star')" > /dev/null
sql -d postgres -c "DROP DATABASE scd_b1_legacy_star WITH (FORCE)" -c "DROP DATABASE scd_b1_legacy_main WITH (FORCE)" > /dev/null
for s in $hub_subs; do resume "${s%%:*}" "${s##*:}"; done
for n in $nodes; do
  wait_for "scd_b1_$n" "SELECT bool_and(pid IS NOT NULL) FROM pg_catalog.pg_stat_subscription WHERE relid IS NULL AND leader_pid IS NULL" "apply workers up again on $n"
done
