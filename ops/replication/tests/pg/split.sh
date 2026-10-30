say "B2: test kinds (same body on every node)"
for n in $nodes; do
  sql -d "scd_b1_$n" -f - <<'SQL'
CREATE OR REPLACE FUNCTION public.scd_cmd_track_edit(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
DECLARE
    t uuid;
BEGIN
    IF args ->> 'fail_on' = public.scd_role() THEN
        RAISE EXCEPTION 'test kind refused on %', public.scd_role() USING ERRCODE = 'check_violation';
    END IF;
    UPDATE public.tracks SET title = args ->> 'title', title_normalized = lower(args ->> 'title') WHERE urn = args ->> 'urn' RETURNING id INTO t;
    RETURN CASE WHEN t IS NULL THEN '[]'::jsonb ELSE jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', t))) END;
END
$f$;
CREATE OR REPLACE FUNCTION public.scd_cmd_track_delete(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
DECLARE
    t uuid;
BEGIN
    DELETE FROM public.tracks WHERE urn = args ->> 'urn' RETURNING id INTO t;
    RETURN CASE WHEN t IS NULL THEN '[]'::jsonb ELSE jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', t))) END;
END
$f$;
CREATE OR REPLACE FUNCTION public.scd_cmd_observe_playlists(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
BEGIN
    IF public.scd_role() = 'core' AND args ->> 'core' = 'skip' THEN
        RETURN '[]'::jsonb;
    END IF;
    INSERT INTO public.playlists (urn, sc_playlist_id, title, title_normalized)
    VALUES (args ->> 'urn', args ->> 'sc_id', args ->> 'title', lower(args ->> 'title'))
    ON CONFLICT (urn) DO UPDATE SET title = EXCLUDED.title, title_normalized = EXCLUDED.title_normalized;
    INSERT INTO public.playlist_membership_state (playlist_urn) VALUES (args ->> 'urn') ON CONFLICT DO NOTHING;
    RETURN jsonb_build_array(
        jsonb_build_object('tbl', 'playlists', 'pk', jsonb_build_object('urn', args ->> 'urn')),
        jsonb_build_object('tbl', 'playlist_membership_state', 'pk', jsonb_build_object('playlist_urn', args ->> 'urn')));
END
$f$;
CREATE OR REPLACE FUNCTION public.scd_cmd_sync_queue_revive(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
BEGIN
    UPDATE public.sync_queue SET dead = false, retry_count = 0 WHERE id = (args ->> 'id')::uuid;
    RETURN '[]'::jsonb;
END
$f$;
SQL
done
echo "ok kinds installed"

say "B2 dispatch: an optimistic command from main runs on core inside the apply tx"
sql -d scd_b1_core -c "INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms)
  VALUES ('00000000-0000-0000-0000-00000000e001', 'urn:t:e1', 'e1', 'Old', 'old', 1000)"
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.tracks WHERE urn = 'urn:t:e1')" "core's track reaches main"
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM public.tracks WHERE urn = 'urn:t:e1')" "core's track reaches star"
[ "$(val scd_b1_main "SELECT count(*) FROM public.lyrics_lookup_state WHERE sc_track_id = 'e1'")" = 1 ] \
  || { echo "FAIL the derived lyrics row did not replicate from core"; exit 1; }
val scd_b1_main "SELECT public.scd_wt('track.edit', '{\"urn\": \"urn:t:e1\", \"title\": \"Edited\"}');
  SELECT title FROM public.tracks WHERE urn = 'urn:t:e1';
  SELECT count(*) FROM public.cmd_pending p JOIN public.cmd_outbox o ON o.node = p.node AND o.id = p.id WHERE o.done_at IS NULL;
  SELECT input_title FROM public.lyrics_lookup_state WHERE sc_track_id = 'e1'" > "$tmp/wt"
cid="$(sed -n 1p "$tmp/wt")"
[ "$(sed -n '2,4p' "$tmp/wt" | tr '\n' ' ')" = "Edited 1 Old " ] \
  || { echo "FAIL inside the scd_wt tx on main: $(tr '\n' ' ' < "$tmp/wt")"; exit 1; }
echo "ok main applies the edit at once, holds it pending and does not fire the derived trigger"
wait_for scd_b1_core "SELECT status = 'ok' FROM public.cmd_done WHERE id = '$cid'" "core acks the command"
wait_for scd_b1_main "SELECT done_at IS NOT NULL FROM public.cmd_outbox WHERE id = '$cid'" "main stamps done_at"
wait_for scd_b1_main "SELECT NOT EXISTS (SELECT 1 FROM public.cmd_pending WHERE id = '$cid')" "main clears pending"
wait_for scd_b1_star "SELECT input_title = 'Edited' FROM public.lyrics_lookup_state WHERE sc_track_id = 'e1'" "core's derived row reaches star"
[ "$(val scd_b1_core "SELECT count(*) FROM public.cmd_outbox")" = 0 ] || { echo "FAIL core stored a cmd_outbox row"; exit 1; }
[ "$(val scd_b1_main "SELECT count(*) FROM public.cmd_done")" = 0 ] || { echo "FAIL main stored a cmd_done row"; exit 1; }
settle ee10 ee11 ee12
pairs tracks "urn = 'urn:t:e1'" "track edit equal on all nodes"
pairs lyrics_lookup_state "sc_track_id = 'e1'" "derived lyrics row equal on all nodes"

say "B2 dispatch: a duplicate re-send is a no-op"
sql -d scd_b1_core -c "SET session_replication_role = replica" \
  -c "INSERT INTO public.cmd_outbox (node, id, kind, args, touched) SELECT 'main', '$cid', 'track.edit', '{\"urn\": \"urn:t:e1\", \"title\": \"Again\"}', '[]' FROM generate_series(1, 3)"
[ "$(val scd_b1_core "SELECT count(*) || ' ' || min(title) FROM public.tracks t, public.cmd_done d WHERE t.urn = 'urn:t:e1' AND d.id = '$cid'")" = "1 Edited" ] \
  || { echo "FAIL a re-sent command ran again"; exit 1; }
echo "ok three re-sends of the same (node, id) change nothing"

say "B2 dispatch: a dead command still repairs the api's optimistic row (D6)"
cid="$(val scd_b1_main "SELECT public.scd_wt('track.edit', '{\"urn\": \"urn:t:e1\", \"title\": \"Refused\", \"fail_on\": \"core\"}')")"
wait_for scd_b1_core "SELECT status = 'dead' FROM public.cmd_done WHERE id = '$cid'" "core marks the command dead"
[ "$(val scd_b1_core "SELECT sqlstate FROM public.cmd_dead WHERE id = '$cid'")" = 23514 ]
wait_for scd_b1_main "SELECT title = 'Edited' FROM public.tracks WHERE urn = 'urn:t:e1'" "cmd_reassert puts core's title back on main"
settle ee13 ee14 ee15
pairs tracks "urn = 'urn:t:e1'" "track equal on all nodes after a dead command"

say "B2 dispatch: FK cascade and derived rows inside dispatch"
cid="$(val scd_b1_star "SELECT public.scd_wt('track.delete', '{\"urn\": \"urn:t:e1\"}')")"
wait_for scd_b1_main "SELECT NOT EXISTS (SELECT 1 FROM public.tracks WHERE urn = 'urn:t:e1')
  AND NOT EXISTS (SELECT 1 FROM public.lyrics_lookup_state WHERE sc_track_id = 'e1')" "core's delete and its cascade reach main"
settle ee16 ee17 ee18
pairs tracks "urn = 'urn:t:e1'" "deleted track gone on all nodes"
pairs lyrics_lookup_state "sc_track_id = 'e1'" "cascaded lyrics row gone on all nodes"

say "B2 dispatch: a key core does not create is purged on the api with its children (cmd_purge loop)"
val scd_b1_main "SELECT public.scd_wt('observe.playlists', '{\"urn\": \"urn:p:1\", \"sc_id\": \"p1\", \"title\": \"P\", \"core\": \"skip\"}');
  SELECT count(*) FROM public.playlist_membership_state WHERE playlist_urn = 'urn:p:1'" > "$tmp/wt"
cid="$(sed -n 1p "$tmp/wt")"
[ "$(sed -n 2p "$tmp/wt")" = 1 ] || { echo "FAIL main did not create the optimistic membership row"; exit 1; }
wait_for scd_b1_main "SELECT NOT EXISTS (SELECT 1 FROM public.playlists WHERE urn = 'urn:p:1')
  AND NOT EXISTS (SELECT 1 FROM public.playlist_membership_state WHERE playlist_urn = 'urn:p:1')" "main purges the playlist and its membership row"
[ "$(val scd_b1_core "SELECT count(*) FROM public.cmd_purge WHERE id = '$cid'")" = 2 ]
settle ee19 ee1a ee1b
pairs playlists "urn = 'urn:p:1'" "purged playlist gone on all nodes"
pairs playlist_membership_state "playlist_urn = 'urn:p:1'" "purged membership gone on all nodes"

say "B2 zz_owner: core wins a unique key; the local optimistic row and its children are purged in the loop"
sql -d scd_b1_main -c "SET scd.cmd = '00000000-0000-0000-0000-0000000000f2'" \
  -c "INSERT INTO public.playlists (urn, sc_playlist_id, title, title_normalized) VALUES ('urn:p:opt', 'p2', 'Opt', 'opt')" \
  -c "INSERT INTO public.playlist_membership_state (playlist_urn) VALUES ('urn:p:opt')" \
  -c "INSERT INTO public.playlist_remote_snapshots (id, playlist_urn, content_fingerprint, track_count) VALUES ('00000000-0000-0000-0000-0000000000f3', 'urn:p:opt', sha256('fp'), 0)"
sql -d scd_b1_core -c "INSERT INTO public.playlists (urn, sc_playlist_id, title, title_normalized) VALUES ('urn:p:core', 'p2', 'Core', 'core')"
wait_for scd_b1_main "SELECT array_agg(urn ORDER BY urn) = ARRAY['urn:p:core'] FROM public.playlists WHERE sc_playlist_id = 'p2'
  AND NOT EXISTS (SELECT 1 FROM public.playlist_membership_state WHERE playlist_urn = 'urn:p:opt')
  AND NOT EXISTS (SELECT 1 FROM public.playlist_remote_snapshots WHERE playlist_urn = 'urn:p:opt')" "main purged the optimistic playlist, its membership and its snapshot"
settle ee1c ee1d ee1e
pairs playlists "sc_playlist_id = 'p2'" "playlist p2 equal on all nodes"
pairs playlist_remote_snapshots "true" "snapshots equal on all nodes"

say "B2 non-optimistic kind: main ships it, only core runs it"
sql -d scd_b1_core -c "INSERT INTO public.sync_queue (id, user_id, action_type, target_urn, dead, retry_count) VALUES ('00000000-0000-0000-0000-00000000a001', 'q1', 'like', 'urn:q:1', true, 7)"
wait_for scd_b1_main "SELECT dead FROM public.sync_queue WHERE id = '00000000-0000-0000-0000-00000000a001'" "the dead row reaches main"
cid="$(val scd_b1_main "SELECT public.scd_wt('sync_queue.revive', '{\"id\": \"00000000-0000-0000-0000-00000000a001\"}')")"
[ "$(val scd_b1_main "SELECT touched FROM public.cmd_outbox WHERE id = '$cid'")" = "[]" ]
wait_for scd_b1_star "SELECT NOT dead AND retry_count = 0 FROM public.sync_queue WHERE id = '00000000-0000-0000-0000-00000000a001'" "core's revive reaches star"
settle ee1f ee20 ee21
pairs sync_queue "user_id = 'q1'" "revived row equal on all nodes"

say "B2 R6: one natural key born on main and star"
pause star star_from_core
pause core core_from_star
sql -d scd_b1_main -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES ('00000000-0000-0000-0000-00000000b001', 'r6', 'same')"
sql -d scd_b1_star -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES ('00000000-0000-0000-0000-00000000b001', 'r6', 'same')"
sql -d scd_b1_main -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES ('00000000-0000-0000-0000-00000000b002', 'r6', 'other')"
sql -d scd_b1_star -c "INSERT INTO public.disliked_tracks (id, sc_user_id, sc_track_id) VALUES ('00000000-0000-0000-0000-00000000b003', 'r6', 'other')"
resume star star_from_core
resume core core_from_star
settle ee22 ee23 ee24
pairs disliked_tracks "sc_user_id = 'r6' AND sc_track_id = 'same'" "same id: one row, equal on all nodes"
[ "$(val scd_b1_core "SELECT n FROM public.scd_counters WHERE counter = 'conflict' AND tbl = 'disliked_tracks'")" -ge 1 ]
[ "$(val scd_b1_star "SELECT n FROM public.scd_counters WHERE counter = 'conflict' AND tbl = 'disliked_tracks'")" -ge 1 ]
for n in $nodes; do
  val "scd_b1_$n" "SELECT count(*) || ' ' || (public.scd_row_json('disliked_tracks', jsonb_build_object('id', min(id::text)::uuid)) - 'id')::text
    FROM public.disliked_tracks WHERE sc_user_id = 'r6' AND sc_track_id = 'other'" > "$tmp/r6.$n"
done
diff -q "$tmp/r6.core" "$tmp/r6.main" > /dev/null && diff -q "$tmp/r6.core" "$tmp/r6.star" > /dev/null \
  || { echo "FAIL different ids: content differs"; cat "$tmp"/r6.*; exit 1; }
echo "ok different ids: one row per node, content equal, conflict counted on core and star; ids: core=$(val scd_b1_core "SELECT id FROM public.disliked_tracks WHERE sc_track_id = 'other'") star=$(val scd_b1_star "SELECT id FROM public.disliked_tracks WHERE sc_track_id = 'other'")"

say "B2 D2: a like born on main and star merges created_at with least"
pause star star_from_core
pause core core_from_star
sql -d scd_b1_main -c "INSERT INTO public.user_likes_tracks (user_id, sc_track_id, created_at) VALUES ('d2', 'a', '2026-03-01'), ('d2', 'b', '2026-01-01')"
sql -d scd_b1_star -c "INSERT INTO public.user_likes_tracks (user_id, sc_track_id, created_at, wanted_state) VALUES ('d2', 'a', '2026-01-01', false), ('d2', 'b', '2026-03-01', false)"
resume star star_from_core
resume core core_from_star
settle ee25 ee26 ee27
pairs user_likes_tracks "user_id = 'd2'" "concurrently born likes equal on all nodes"
[ "$(val scd_b1_core "SELECT string_agg(sc_track_id || ' ' || created_at::date || ' ' || wanted_state, ', ' ORDER BY sc_track_id) FROM public.user_likes_tracks WHERE user_id = 'd2'")" \
  = "a 2026-01-01 false, b 2026-01-01 false" ] || { echo "FAIL D2 merge result"; exit 1; }
echo "ok created_at is the earliest birth and the newer intent (star) wins"

say "B2 zz_fk: core refuses an api delete that would orphan a RESTRICT child, and re-sends the parent"
sql -d scd_b1_core -c "INSERT INTO public.oauth_apps (id, name, client_id, client_secret, redirect_uri) VALUES ('00000000-0000-0000-0000-00000000c001', 'app', 'cid-1', 's', 'http://x')"
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM public.oauth_apps WHERE client_id = 'cid-1')" "the app reaches star"
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.oauth_apps WHERE client_id = 'cid-1')" "the app reaches main"
pause main main_from_core
sql -d scd_b1_star -c "INSERT INTO public.soundcloud_connections (id, soundcloud_user_id, access_token, refresh_token, expires_at, scope, oauth_app_id)
  VALUES ('00000000-0000-0000-0000-00000000c002', 'u-fk', 'a', 'r', now() + interval '1 hour', 's', '00000000-0000-0000-0000-00000000c001')"
wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM public.soundcloud_connections WHERE soundcloud_user_id = 'u-fk')" "core has star's connection"
sql -d scd_b1_main -c "DELETE FROM public.oauth_apps WHERE client_id = 'cid-1'"
wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM public.repl_resend WHERE tbl = 'oauth_apps')" "core refuses and queues a resend"
[ "$(val scd_b1_core "SELECT n FROM public.scd_counters WHERE counter = 'restrict' AND tbl = 'oauth_apps'")" = 1 ]
[ "$(val scd_b1_core "SELECT public.scd_drain_resend()")" = 1 ]
resume main main_from_core
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.oauth_apps WHERE client_id = 'cid-1')" "main gets the app back through repl_repair"
settle ee28 ee29 ee2a
pairs oauth_apps "client_id = 'cid-1'" "app equal on all nodes"
pairs soundcloud_connections "soundcloud_user_id = 'u-fk'" "connection equal on all nodes"
[ "$(val scd_b1_core "SELECT count(*) FROM public.repl_repair")" = 0 ] || { echo "FAIL core kept a repl_repair row"; exit 1; }

say "B2 zz_fk cascade and zz_orphan count"
sql -d scd_b1_core -c "INSERT INTO public.oauth_apps (id, name, client_id, client_secret, redirect_uri) VALUES ('00000000-0000-0000-0000-00000000c003', 'app2', 'cid-2', 's', 'http://x')" \
  -c "INSERT INTO public.oauth_app_tokens (oauth_app_id, access_token, expires_at) VALUES ('00000000-0000-0000-0000-00000000c003', 't', now() + interval '1 hour')"
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.oauth_apps WHERE client_id = 'cid-2')" "app2 reaches main"
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM public.oauth_apps WHERE client_id = 'cid-2')" "app2 reaches star"
pause core core_from_star
sql -d scd_b1_star -c "INSERT INTO public.soundcloud_connections (id, soundcloud_user_id, access_token, refresh_token, expires_at, scope, oauth_app_id)
  VALUES ('00000000-0000-0000-0000-00000000c004', 'u-orphan', 'a', 'r', now() + interval '1 hour', 's', '00000000-0000-0000-0000-00000000c003')"
sql -d scd_b1_main -c "DELETE FROM public.oauth_apps WHERE client_id = 'cid-2'"
wait_for scd_b1_core "SELECT NOT EXISTS (SELECT 1 FROM public.oauth_apps WHERE client_id = 'cid-2')
  AND NOT EXISTS (SELECT 1 FROM public.oauth_app_tokens WHERE oauth_app_id = '00000000-0000-0000-0000-00000000c003')" "core applies the delete and its cascade"
resume core core_from_star
wait_for scd_b1_core "SELECT n = 1 FROM public.scd_counters WHERE counter = 'orphan' AND tbl = 'soundcloud_connections'" "core counts the orphaned connection"
settle ee2b ee2c ee2d
pairs oauth_apps "client_id = 'cid-2'" "deleted app gone on all nodes"
pairs oauth_app_tokens "oauth_app_id = '00000000-0000-0000-0000-00000000c003'" "cascaded tokens gone on all nodes"
pairs soundcloud_connections "soundcloud_user_id = 'u-orphan'" "orphaned connection kept and equal on all nodes"

say "B2 resend drain on an api node: a refused purge comes back to core and the other api node"
[ "$(val scd_b1_main "SELECT public.scd_drain_resend()")" -ge 1 ]
wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM public.user_likes_tracks WHERE user_id = 'r3')" "core re-inserts main's kept like"
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM public.user_likes_tracks WHERE user_id = 'r3')" "star gets it from core"
[ "$(val scd_b1_main "SELECT count(*) FROM public.repl_resend")" = 0 ]
[ "$(val scd_b1_main "SELECT count(*) FROM public.repl_repair")" = 0 ]
settle ee2e ee2f ee30
pairs user_likes_tracks "user_id IN ('r2', 'r3')" "re-sent like equal on all nodes"

for t in tracks playlists playlist_membership_state playlist_remote_snapshots lyrics_lookup_state sync_queue user_likes_tracks oauth_apps oauth_app_tokens soundcloud_connections; do
  pairs "$t" true "whole table $t"
done
pairs disliked_tracks "sc_track_id <> 'other'" "whole table disliked_tracks except the different-id R6 key"

say "B2 scd_unpublish_column drops the column from the local lists, sync stays idempotent"
val scd_b1_main "SELECT public.scd_unpublish_column('user_events', 'position_pct')" > /dev/null
[ "$(val scd_b1_main "SELECT 'position_pct' = ANY (attnames) FROM pg_catalog.pg_publication_tables WHERE pubname = 'scd_api_pub' AND tablename = 'user_events'")" = f ]
[ "$(val scd_b1_main "SELECT count(*) FROM public.scd_sync_publications()")" = 0 ]
if sql -d scd_b1_main -c "SELECT public.scd_unpublish_column('user_events', 'weight')" 2> "$tmp/unpub.err"; then
  echo "FAIL a NOT NULL column without a default was unpublished"; exit 1
fi
echo "ok user_events.position_pct unpublished on main; a NOT NULL column without a default is refused"
