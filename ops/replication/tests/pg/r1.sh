long_wait() {
  local i
  for i in $(seq 1 600); do
    if [ "$(val "$1" "$2")" = t ]; then
      echo "ok $3"
      return 0
    fi
    sleep 0.5
  done
  echo "FAIL $3 (on $1: $2)"
  exit 1
}

node_batch() { only "$1" "SELECT deleted || '|' || redacted || '|' || coalesce(held, '') FROM public.scd_node_retention_batch('$2')"; }

node_drain() {
  local d=0 r=0 out
  while :; do
    out="$(node_batch "$1" "$2")"
    d=$((d + $(echo "$out" | cut -d'|' -f1)))
    r=$((r + $(echo "$out" | cut -d'|' -f2)))
    [ $(( $(echo "$out" | cut -d'|' -f1) + $(echo "$out" | cut -d'|' -f2) )) -lt 1000 ] && break
  done
  echo "$d|$r|$(echo "$out" | cut -d'|' -f3)"
}

core_drain() {
  local n=0 d
  while :; do
    d="$(batch "$1" 1000 | cut -d'|' -f1)"
    n=$((n + d))
    [ "$d" -lt 1000 ] && break
  done
  echo "$n"
}

r1_rows() { only "$1" "SELECT count(*) FROM public.cmd_outbox WHERE args ->> 'r1' = 'true' AND $2"; }
applied_twice() { only core "SELECT count(*) - count(DISTINCT id) FROM r1.applied"; }
ids_of() { only "$1" "SELECT string_agg(quote_literal(id::text), ', ' ORDER BY id) FROM public.cmd_outbox WHERE args ->> 'r1' = 'true' AND $2"; }

repush() {
  sql -d "scd_b1_$1" -c "BEGIN" \
    -c "CREATE TEMP TABLE again ON COMMIT DROP AS SELECT node, id, kind, args, touched, created_at FROM public.cmd_outbox WHERE $2" \
    -c "DELETE FROM public.cmd_outbox o USING again a WHERE o.node = a.node AND o.id = a.id" \
    -c "INSERT INTO public.cmd_outbox (node, id, kind, args, touched, created_at) SELECT node, id, kind, args, touched, created_at FROM again" \
    -c "COMMIT" > /dev/null
}

say "R1: fixtures, and core logs every run of an r1 track.edit or sync_queue.revive"
sql -d scd_b1_core -f - > /dev/null <<'SQL'
CREATE SCHEMA r1;
CREATE TABLE r1.applied (kind text NOT NULL, id uuid NOT NULL);
GRANT USAGE ON SCHEMA r1 TO scd_repl;
GRANT SELECT, INSERT ON r1.applied TO scd_repl;
CREATE OR REPLACE FUNCTION public.scd_cmd_track_edit(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
DECLARE
    t uuid;
BEGIN
    IF args ->> 'fail_on' = public.scd_role() THEN
        RAISE EXCEPTION 'test kind refused on %', public.scd_role() USING ERRCODE = 'check_violation';
    END IF;
    IF args ->> 'r1' = 'true' THEN
        INSERT INTO r1.applied (kind, id) VALUES ('track.edit', current_setting('scd.cmd')::uuid);
    END IF;
    UPDATE public.tracks SET title = args ->> 'title', title_normalized = lower(args ->> 'title') WHERE urn = args ->> 'urn' RETURNING id INTO t;
    IF t IS NULL AND args ->> 'need_row' = 'true' THEN
        RAISE EXCEPTION 'no track %', args ->> 'urn' USING ERRCODE = 'check_violation';
    END IF;
    RETURN jsonb_build_object('output', args ->> 'title',
        'touched', CASE WHEN t IS NULL THEN '[]'::jsonb ELSE jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', t))) END);
END
$f$;
CREATE OR REPLACE FUNCTION public.scd_cmd_sync_queue_revive(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
BEGIN
    IF args ->> 'r1' = 'true' THEN
        INSERT INTO r1.applied (kind, id) VALUES ('sync_queue.revive', current_setting('scd.cmd')::uuid);
    END IF;
    UPDATE public.sync_queue SET dead = false, retry_count = 0 WHERE id = (args ->> 'id')::uuid;
    RETURN '{"touched": []}'::jsonb;
END
$f$;
INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms)
SELECT public.scd_uuid('tracks', 'urn:t:r1:' || n), 'urn:t:r1:' || n, 'r1t' || n, 'R1 ' || n, 'r1 ' || n, 1000 FROM generate_series(1, 50) n;
INSERT INTO public.sync_queue (id, user_id, action_type, target_urn, dead, retry_count)
SELECT public.scd_uuid('sync_queue', 'r1q', 'like', 'urn:q:r1:' || n), 'r1q', 'like', 'urn:q:r1:' || n, true, 3 FROM generate_series(1, 50) n;
INSERT INTO public.oauth_apps (id, name, client_id, client_secret, redirect_uri)
VALUES (public.scd_uuid('oauth_apps', 'r1client'), 'r1', 'r1client', 'r1secret', 'https://r1.test/');
INSERT INTO public.soundcloud_connections (id, soundcloud_user_id, username, oauth_app_id, access_token, refresh_token, expires_at, scope)
SELECT c, 'r1u' || c, 'r1', public.scd_uuid('oauth_apps', 'r1client'), 'r1-a-0', 'r1-r-0', now() + interval '1 hour', ''
FROM unnest(ARRAY['20000000-0000-4000-8000-0000000001a1', '20000000-0000-4000-8000-0000000001a2',
                  '20000000-0000-4000-8000-0000000001b1', '20000000-0000-4000-8000-0000000001b2']::uuid[]) c;
SQL
for n in main star; do
  wait_for "scd_b1_$n" "SELECT (SELECT count(*) FROM public.soundcloud_connections WHERE username = 'r1') = 4
    AND (SELECT count(*) FROM public.tracks WHERE urn LIKE 'urn:t:r1:%') = 50
    AND (SELECT count(*) FROM public.sync_queue WHERE user_id = 'r1q') = 50" "the r1 fixtures reach $n"
done

say "R1: 10000 commands from main and star (track.edit, some refused on core, sync_queue.revive, observe.playlists purged by core, connection.login)"
t0="$(only core "SELECT now()")"
for round in 1 2 3 4 5; do
  for n in main star; do
    sql -d "scd_b1_$n" -c "SET r1.round = '$round'" -f - > /dev/null <<'SQL'
DO $f$
DECLARE
    round int := current_setting('r1.round')::int;
    me text := public.scd_node();
    app uuid := public.scd_uuid('oauth_apps', 'r1client');
    conns uuid[] := CASE public.scd_node()
        WHEN 'main' THEN ARRAY['20000000-0000-4000-8000-0000000001a1', '20000000-0000-4000-8000-0000000001a2']::uuid[]
        ELSE ARRAY['20000000-0000-4000-8000-0000000001b1', '20000000-0000-4000-8000-0000000001b2']::uuid[]
    END;
    i int;
    c uuid;
    v bigint;
BEGIN
    FOR i IN 1..1000 LOOP
        IF i % 5 < 3 THEN
            PERFORM public.scd_wt('track.edit', jsonb_build_object('r1', true, 'urn', 'urn:t:r1:' || (i % 50 + 1),
                'title', format('R1 %s %s %s', me, round, i), 'fail_on', CASE WHEN i % 100 = 0 THEN 'core' END));
        ELSIF i % 5 = 3 THEN
            PERFORM public.scd_wt('sync_queue.revive', jsonb_build_object('r1', true,
                'id', public.scd_uuid('sync_queue', 'r1q', 'like', 'urn:q:r1:' || (i % 50 + 1))));
        ELSIF i % 50 = 4 THEN
            PERFORM public.scd_wt('observe.playlists', jsonb_build_object('r1', true, 'urn', format('urn:p:r1:%s:%s:%s', me, round, i),
                'sc_id', format('r1p-%s-%s-%s', me, round, i), 'title', 'R1', 'core', 'skip'));
        ELSE
            c := conns[1 + i % 2];
            SELECT public.scd_hlc(s.token_v) INTO v FROM public.soundcloud_connections s WHERE s.id = c;
            PERFORM public.scd_wt('connection.login', jsonb_build_object('r1', true, 'connection_id', c,
                'soundcloud_user_id', 'r1u' || c, 'username', 'r1', 'oauth_app_id', app,
                'access_token', format('r1-a-%s-%s', round, i), 'refresh_token', format('r1-r-%s-%s', round, i),
                'expires_at', now() + interval '1 hour', 'scope', '', 'login_v', v, 'at', clock_timestamp()));
        END IF;
    END LOOP;
END
$f$;
SQL
  done
done
want "main wrote 5000 commands" "$(r1_rows main true)" 5000
want "star wrote 5000 commands" "$(r1_rows star true)" 5000
long_wait scd_b1_main "SELECT NOT EXISTS (SELECT 1 FROM public.cmd_outbox WHERE args ->> 'r1' = 'true' AND done_at IS NULL)" "main got every ack"
long_wait scd_b1_star "SELECT NOT EXISTS (SELECT 1 FROM public.cmd_outbox WHERE args ->> 'r1' = 'true' AND done_at IS NULL)" "star got every ack"
want "core ran each command once (cmd_done rows)" "$(only core "SELECT count(*) FROM public.cmd_done WHERE at >= '$t0'")" 10000
want "the logged runs: 2950 edits and 1000 revives per node" "$(only core "SELECT count(*) FROM r1.applied")" 7900
want "no command ran twice" "$(applied_twice)" 0
want "the refused edits are dead letters" "$(only core "SELECT count(*) FROM public.cmd_dead WHERE at >= '$t0' AND message = 'test kind refused on core'")" 100
want "core answered 200 skipped playlists with 400 purges" "$(only core "SELECT count(*) FROM public.cmd_purge p JOIN public.cmd_done d USING (node, id) WHERE d.at >= '$t0'")" 400
want "core reasserted rows (refused and raced edits)" "$(only core "SELECT count(*) >= 100 FROM public.cmd_reassert r JOIN public.cmd_done d USING (node, id) WHERE d.at >= '$t0'")" t
settle ee51 ee52 ee53
pairs tracks "urn LIKE 'urn:t:r1:%'" "r1 tracks equal on all nodes"
pairs sync_queue "user_id = 'r1q'" "r1 sync_queue rows equal on all nodes"
pairs playlists "urn LIKE 'urn:p:r1:%'" "purged r1 playlists gone on all nodes"
pairs soundcloud_connections "username = 'r1'" "r1 connections equal on all nodes"

say "R1: api rows get ages: 2000 acked 3 days ago, 400 logins acked 2 hours ago, 150 lost acks (100 from 3 days, 50 from 8 days)"
for n in main star; do
  sql -d "scd_b1_$n" -f - > /dev/null <<'SQL'
CREATE TEMP TABLE pick AS
SELECT id, kind, row_number() OVER (PARTITION BY kind ORDER BY id) AS n
FROM public.cmd_outbox
WHERE args ->> 'r1' = 'true' AND args ->> 'fail_on' IS NULL AND kind IN ('track.edit', 'connection.login', 'sync_queue.revive');
UPDATE public.cmd_outbox o SET done_at = NULL, created_at = o.created_at - interval '8 days' FROM pick p
WHERE o.id = p.id AND ((p.kind = 'track.edit' AND p.n <= 40) OR (p.kind = 'connection.login' AND p.n <= 10));
UPDATE public.cmd_outbox o SET done_at = NULL, created_at = o.created_at - interval '3 days' FROM pick p
WHERE o.id = p.id AND ((p.kind = 'track.edit' AND p.n BETWEEN 41 AND 120) OR (p.kind = 'connection.login' AND p.n BETWEEN 11 AND 30));
UPDATE public.cmd_outbox o SET done_at = o.done_at - interval '2 hours', created_at = o.created_at - interval '2 hours' FROM pick p
WHERE o.id = p.id AND p.kind = 'connection.login' AND p.n BETWEEN 31 AND 430;
UPDATE public.cmd_outbox o SET done_at = o.done_at - interval '3 days', created_at = o.created_at - interval '3 days' FROM pick p
WHERE o.id = p.id AND ((p.kind = 'track.edit' AND p.n BETWEEN 121 AND 1620) OR (p.kind = 'sync_queue.revive' AND p.n <= 500));
SQL
  want "$n: 150 lost acks, 2000 old acks, 400 old login acks" \
    "$(r1_rows "$n" "done_at IS NULL")/$(r1_rows "$n" "done_at < now() - interval '2 days'")/$(r1_rows "$n" "kind = 'connection.login' AND done_at < now() - interval '1 hour'")" 150/2000/400
done

say "R1 node retention guard: an api node holds while its stream from core or core's read of it stops"
pause main main_from_core
want "a disabled subscription from core holds main" "$(node_batch main cmd_outbox)" "0|0|main_from_core: subscription disabled"
resume main main_from_core
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = 'main_from_core' AND relid IS NULL AND leader_pid IS NULL AND pid IS NOT NULL AND last_msg_receipt_time IS NOT NULL)" "main reads core again"
pause core core_from_star
wait_for postgres "SELECT NOT active FROM pg_catalog.pg_replication_slots WHERE slot_name = 'scd_b1_core_from_star'" "core stopped reading star's slot"
want "an inactive slot of core holds star" "$(node_batch star cmd_outbox)" "0|0|scd_b1_core_from_star: slot inactive"
resume core core_from_star
wait_for postgres "SELECT active FROM pg_catalog.pg_replication_slots WHERE slot_name = 'scd_b1_core_from_star'" "core reads star's slot again"
want "a silent stream holds (stale limit forced into the future)" "$(only main "SELECT count(*) FROM public.scd_node_retention_guard(stale => interval '-1 hour') WHERE reason LIKE 'no message from core for %'")" 1
want "a lag over the limit holds (limit forced below zero)" "$(only main "SELECT count(*) > 0 FROM public.scd_node_retention_guard(lag_bytes => -1) WHERE reason LIKE '% bytes behind'")" t
want "the guard is clear on main" "$(only main "SELECT coalesce(string_agg(peer || ': ' || reason, '; '), 'clear') FROM public.scd_node_retention_guard()")" clear
want "the guard is clear on star" "$(only star "SELECT coalesce(string_agg(peer || ': ' || reason, '; '), 'clear') FROM public.scd_node_retention_guard()")" clear
want "core runs node retention without the api guard, its outbox is empty" "$(node_batch core cmd_outbox)" "0|0|"
want "nothing moved while held" "$(r1_rows main "done_at < now() - interval '2 days'")/$(r1_rows star "done_at < now() - interval '2 days'")" 2000/2000

say "R1 node retention: deletes acked rows past 2 days, redacts login pairs, keeps every pending row"
for n in main star; do
  want "$n: deleted, redacted, held for lost acks" "$(node_drain "$n" cmd_outbox)" "2000|410|150 commands wait for core longer than 2 days"
  want "$n: rows left, pending kept" "$(r1_rows "$n" true)/$(r1_rows "$n" "done_at IS NULL")" 3000/150
  want "$n: no acked row older than 2 days" "$(r1_rows "$n" "done_at < now() - interval '2 days'")" 0
  want "$n: no token pair in a row acked over an hour ago or pending past 7 days" \
    "$(r1_rows "$n" "args ?| ARRAY['access_token', 'refresh_token'] AND (done_at < now() - interval '1 hour' OR (done_at IS NULL AND created_at < now() - interval '7 days'))")" 0
  want "$n: young and lost-ack logins keep their pair, redacted ones keep the rest of the args" \
    "$(r1_rows "$n" "kind = 'connection.login' AND args ?| ARRAY['access_token', 'refresh_token']")/$(r1_rows "$n" "args ->> 'redacted' = 'true' AND args ->> 'connection_id' IS NOT NULL")" 490/410
  want "$n: a second run moves nothing" "$(node_batch "$n" cmd_outbox | cut -d'|' -f1,2)" "0|0"
done
sql -d scd_b1_main -c "INSERT INTO public.oauth_app_tokens_local (oauth_app_id, access_token, expires_at) VALUES
  ('40000000-0000-4000-8000-0000000001d1', 'r1-gone', now() - interval '2 hours'),
  ('40000000-0000-4000-8000-0000000001d2', 'r1-valid', now() + interval '1 hour')" > /dev/null
want "expired local app tokens go after an hour" "$(node_batch main oauth_app_tokens_local)/$(only main "SELECT count(*) FROM public.oauth_app_tokens_local WHERE oauth_app_id::text LIKE '40000000-0000-4000-8000-0000000001d%'")" "1|0|/1"

say "R1 core retention: reasserts and purges go with their ack, only while the guard is clear"
only core "DELETE FROM public.cmd_done WHERE node = 'ret'" > /dev/null
want "no old ack is left from earlier checks" "$(only core "SELECT count(*) FROM public.cmd_done WHERE at < now() - interval '7 days'")" 0
for n in main star; do
  aged="$(ids_of "$n" "created_at < now() - interval '7 days'")"
  sql -d scd_b1_core -c "UPDATE public.cmd_done SET at = at - interval '8 days' WHERE node = '$n' AND id IN ($aged)" > /dev/null
done
sql -d scd_b1_core -c "UPDATE public.cmd_done d SET at = d.at - interval '8 days' WHERE d.at >= '$t0'
  AND (EXISTS (SELECT 1 FROM public.cmd_purge p WHERE p.node = d.node AND p.id = d.id)
       OR EXISTS (SELECT 1 FROM public.cmd_dead x WHERE x.node = d.node AND x.id = d.id AND x.message = 'test kind refused on core'))" > /dev/null
children() { only core "SELECT (SELECT count(*) FROM public.cmd_reassert r JOIN public.cmd_done d USING (node, id) WHERE d.at $1 now() - interval '7 days')
  || '/' || (SELECT count(*) FROM public.cmd_purge p JOIN public.cmd_done d USING (node, id) WHERE d.at $1 now() - interval '7 days')"; }
old_children="$(children '<')"
young_children="$(children '>=')"
want "400 acks are old (100 refused, 200 purged, 100 lost acks of 8 days)" "$(only core "SELECT count(*) FROM public.cmd_done WHERE at < now() - interval '7 days'")" 400
want "the old acks have purges" "$(echo "$old_children" | cut -d/ -f2)" 400
quiet
report_both 0 0
pause star star_from_core
wait_for postgres "SELECT NOT active FROM pg_catalog.pg_replication_slots WHERE slot_name = 'scd_b1_star_from_core'" "core's slot for star is inactive"
for t in cmd_reassert cmd_purge cmd_done; do
  want "$t is held by the inactive slot" "$(held "$t" | grep -c 'scd_b1_star_from_core: slot inactive')" 1
done
want "nothing went while held" "$(children '<')" "$old_children"
resume star star_from_core
wait_for postgres "SELECT active FROM pg_catalog.pg_replication_slots WHERE slot_name = 'scd_b1_star_from_core'" "core's slot for star is active again"
quiet
report_both 0 0
want "a lag over the limit holds the reasserts" "$(only core "SELECT held LIKE '%bytes behind%' FROM public.scd_retention_batch('cmd_reassert', lag_bytes => -1)")" t
want "reasserts of old acks deleted" "$(core_drain cmd_reassert)" "$(echo "$old_children" | cut -d/ -f1)"
want "purges of old acks deleted" "$(core_drain cmd_purge)" 400
want "then the 400 old acks" "$(core_drain cmd_done)" 400
want "no orphan reassert or purge" "$(only core "SELECT count(*) FROM public.cmd_reassert r WHERE NOT EXISTS (SELECT 1 FROM public.cmd_done d WHERE d.node = r.node AND d.id = r.id)")/$(only core "SELECT count(*) FROM public.cmd_purge p WHERE NOT EXISTS (SELECT 1 FROM public.cmd_done d WHERE d.node = p.node AND d.id = p.id)")" 0/0
want "children of young acks untouched" "$(children '>=')" "$young_children"
reach

say "R1 dead letters: a dead login keeps its pair only while a replay can still install it"
core_pair() { only core "SELECT md5(access_token || refresh_token) || ' ' || login_v FROM public.soundcloud_connections WHERE id = '20000000-0000-4000-8000-0000000001a1'"; }
before="$(core_pair)"
lv="$(echo "$before" | cut -d' ' -f2)"
sql -d scd_b1_core -c "INSERT INTO public.cmd_dead (node, id, kind, args, sqlstate, message)
  SELECT 'r1x', l.id, 'connection.login', jsonb_build_object('connection_id', l.conn, 'soundcloud_user_id', 'r1u' || l.conn, 'username', 'r1',
      'oauth_app_id', public.scd_uuid('oauth_apps', 'r1client'), 'access_token', 'r1-dead-a', 'refresh_token', 'r1-dead-r',
      'expires_at', now() + interval '1 hour', 'scope', '', 'login_v', l.v, 'at', now()), '23503', 'seed'
  FROM (VALUES ('00000000-0000-4000-8000-0000000001e1'::uuid, '20000000-0000-4000-8000-0000000001a1', 1::bigint),
               ('00000000-0000-4000-8000-0000000001e2'::uuid, '20000000-0000-4000-8000-0000000001a1', $lv + 16000),
               ('00000000-0000-4000-8000-0000000001e3'::uuid, '20000000-0000-4000-8000-0000000001ff', $lv + 16000)) AS l (id, conn, v)" > /dev/null
want "superseded and orphan letters redacted" "$(only core "SELECT public.scd_dead_redact()")" 2
want "only the installable letter keeps its pair" "$(only core "SELECT string_agg(right(id::text, 2) || '=' || (args ?| ARRAY['access_token', 'refresh_token']), ' ' ORDER BY id) FROM public.cmd_dead WHERE node = 'r1x'")" "e1=false e2=true e3=false"
want "the dead r1 track edits keep their args" "$(only core "SELECT count(*) FROM public.cmd_dead WHERE message = 'test kind refused on core' AND args ->> 'r1' = 'true' AND args ? 'title'")" 100
want "a redacted login refuses to replay" "$(only core "SELECT public.scd_replay_dead('r1x', '00000000-0000-4000-8000-0000000001e1')")" "failed 22023: connection.login of 20000000-0000-4000-8000-0000000001a1: the token pair is missing or was redacted"
want "core's pair did not change" "$(core_pair)" "$before"
want "the installable letter replays" "$(only core "SELECT public.scd_replay_dead('r1x', '00000000-0000-4000-8000-0000000001e2')")" ok
want "core installed its login_v" "$(core_pair | cut -d' ' -f2)" "$((lv + 16000))"
only core "DELETE FROM public.cmd_dead WHERE node = 'r1x'" > /dev/null
settle ee54 ee55 ee56
pairs soundcloud_connections "username = 'r1'" "r1 connections equal on all nodes after the replay"

say "R1 lost core->api slot: acks and reasserts of the gap are lost, a re-push gets them again"
pause main main_from_core
gap="$(only main "SELECT string_agg(quote_literal(w.id::text), ', ') FROM generate_series(1, 10) n, LATERAL public.scd_wt('track.edit', jsonb_build_object('r1', true, 'urn', 'urn:t:r1:' || n, 'title', 'R1 gap ' || n, 'fail_on', CASE WHEN n = 10 THEN 'core' END)) w")"
wait_for scd_b1_core "SELECT count(*) = 10 FROM public.cmd_done WHERE node = 'main' AND id IN ($gap)" "core ran the 10 gap commands"
sql -d scd_b1_main -c "ALTER SUBSCRIPTION main_from_core SET (slot_name = NONE)" -c "DROP SUBSCRIPTION main_from_core" > /dev/null
val scd_b1_core "SELECT pg_catalog.pg_drop_replication_slot('scd_b1_main_from_core')" > /dev/null
subscribe main core scd_core_pub "origin = any, disable_on_error = false, streaming = parallel"
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = 'main_from_core' AND pid IS NOT NULL)" "main reads core from a new slot"
[ "$(val scd_b1_core "SELECT public.scd_resend_all() > 0")" = t ]
swept
want "the gap commands stay pending on main and hold their keys" "$(r1_rows main "id IN ($gap) AND done_at IS NULL")/$(only main "SELECT count(*) FROM public.cmd_pending WHERE id IN ($gap)")" 10/10
repush main "id IN ($gap)"
wait_for scd_b1_main "SELECT NOT EXISTS (SELECT 1 FROM public.cmd_pending WHERE id IN ($gap)) AND NOT EXISTS (SELECT 1 FROM public.cmd_outbox WHERE id IN ($gap) AND done_at IS NULL)" "core acks the re-pushed commands again and main releases the keys"
want "core ran each gap command once, the refused one never" "$(only core "SELECT count(*) || '/' || count(DISTINCT id) FROM r1.applied WHERE id IN ($gap)")" 9/9
refused="$(only core "SELECT title FROM public.tracks WHERE urn = 'urn:t:r1:10'")"
want "core kept its title of the refused gap edit" "$([ "$refused" != 'R1 gap 10' ] && echo kept)" kept
wait_for scd_b1_main "SELECT title = '$refused' FROM public.tracks WHERE urn = 'urn:t:r1:10'" "the re-ack carries the lost reassert, main takes core's title back"
sql -d scd_b1_core -c "UPDATE public.tracks SET title = 'R1 core after gap', title_normalized = 'r1 core after gap' WHERE urn = 'urn:t:r1:1'" > /dev/null
wait_for scd_b1_main "SELECT title = 'R1 core after gap' FROM public.tracks WHERE urn = 'urn:t:r1:1'" "a later core write of a released key reaches main"

say "R1 lost api->core slot: commands of the gap never reach core, a re-push runs each once"
pause core core_from_main
lost="$(only main "SELECT string_agg(quote_literal(w.id::text), ', ') FROM generate_series(11, 20) n, LATERAL public.scd_wt('track.edit', jsonb_build_object('r1', true, 'urn', 'urn:t:r1:' || n, 'title', 'R1 lost ' || n)) w")"
sql -d scd_b1_core -c "ALTER SUBSCRIPTION core_from_main SET (slot_name = NONE)" -c "DROP SUBSCRIPTION core_from_main" > /dev/null
val scd_b1_main "SELECT pg_catalog.pg_drop_replication_slot('scd_b1_core_from_main')" > /dev/null
subscribe core main "scd_api_pub, scd_cmd_pub" "origin = none"
wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_subscription WHERE subname = 'core_from_main' AND pid IS NOT NULL)" "core reads main from a new slot"
[ "$(val scd_b1_main "SELECT public.scd_resend_all() > 0")" = t ]
swept
want "core never saw the lost commands" "$(only core "SELECT count(*) FROM public.cmd_done WHERE id IN ($lost)")" 0
repush main "id IN ($lost)"
wait_for scd_b1_main "SELECT NOT EXISTS (SELECT 1 FROM public.cmd_outbox WHERE id IN ($lost) AND done_at IS NULL)" "core runs and acks the re-pushed lost commands"
want "core ran each lost command once" "$(only core "SELECT count(*) || '/' || count(DISTINCT id) FROM r1.applied WHERE id IN ($lost)")" 10/10

say "R1 re-push of every lost ack: young ones are acked again, ones past the dedup window go to cmd_dead, nothing runs twice"
runs_before="$(only core "SELECT count(*) FROM r1.applied")"
for n in main star; do repush "$n" "args ->> 'r1' = 'true' AND done_at IS NULL AND created_at < now() - interval '2 days'"; done
for n in main star; do
  wait_for "scd_b1_$n" "SELECT NOT EXISTS (SELECT 1 FROM public.cmd_outbox WHERE args ->> 'r1' = 'true' AND done_at IS NULL)" "$n has no pending r1 command left"
done
want "no re-pushed command ran" "$(only core "SELECT count(*) FROM r1.applied")" "$runs_before"
want "no command ran twice" "$(applied_twice)" 0
want "the 100 commands past the window are dead letters" "$(only core "SELECT count(*) FROM public.cmd_dead WHERE message LIKE '%older than the dedup window%'")" 100
want "their 20 logins were redacted on the api, so the letters hold no pair" "$(only core "SELECT count(*) FILTER (WHERE kind = 'connection.login') || '/' || count(*) FILTER (WHERE args ?| ARRAY['access_token', 'refresh_token']) FROM public.cmd_dead WHERE message LIKE '%older than the dedup window%'")" 20/0
for n in main star; do
  want "$n: node retention is no longer held" "$(node_batch "$n" cmd_outbox | cut -d'|' -f3)" ""
done
settle ee57 ee58 ee59
pairs tracks "urn LIKE 'urn:t:r1:%'" "r1 tracks equal on all nodes after the rebuilds and re-pushes"
pairs sync_queue "user_id = 'r1q'" "r1 sync_queue rows equal on all nodes after the rebuilds"
pairs soundcloud_connections "username = 'r1'" "r1 connections equal on all nodes after the rebuilds"
only core "DELETE FROM public.cmd_dead WHERE args ->> 'r1' = 'true' AND (message LIKE '%older than the dedup window%' OR message = 'test kind refused on core')" > /dev/null
