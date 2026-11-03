retire_sql="$(cat "$root/jobs/queries/playlist_observe/retire_remotely_deleted.sql")"
old_retire_sql="$(printf '%s\n' "$retire_sql" | python3 -c "
import sys
s = sys.stdin.read()
a = s.index('cancelled AS (')
b = s.index(', retired AS')
s = s[:a] + '''cancelled AS (
    DELETE FROM sync_queue AS queued
    USING gone
    WHERE queued.user_id = \$2 AND queued.target_urn = gone.urn
)''' + s[b:]
s = s.replace('CROSS JOIN cancelled', '')
print(s)
")"
queued_on() { only "$1" "SELECT count(*) FROM public.sync_queue WHERE target_urn = 'urn:p:$2'"; }
owned_on() { only "$1" "SELECT count(*) FROM public.user_owned_playlists WHERE playlist_urn = 'urn:p:$2'"; }
retire() {
  sql -d "scd_b1_$1" -c "SET scd.guard = 'enforce'" -c "PREPARE retire(text, text, text[]) AS $2" \
    -c "EXECUTE retire('urn:p:$3', 'own-$3', ARRAY['own-$3'])" > /dev/null
}

say "split deletes: the playlist retire query purges the queued intent with a receipt under enforce"
sql -d scd_b1_core -c "INSERT INTO public.playlists (urn, sc_playlist_id, title, title_normalized, owner_sc_user_id) VALUES ('urn:p:dl1', 'dl1', 'dl1', 'dl1', 'own-dl1')" \
  -c "INSERT INTO public.user_owned_playlists (user_id, playlist_urn) VALUES ('own-dl1', 'urn:p:dl1')" > /dev/null
sql -d scd_b1_main -c "INSERT INTO public.sync_queue (id, user_id, action_type, target_urn) VALUES ('00000000-0000-0000-0000-0000000d0001', 'own-dl1', 'playlist_update', 'urn:p:dl1')" > /dev/null
for n in core main star; do
  wait_for "scd_b1_$n" "SELECT (SELECT count(*) FROM public.sync_queue WHERE target_urn = 'urn:p:dl1') = 1 AND (SELECT count(*) FROM public.user_owned_playlists WHERE playlist_urn = 'urn:p:dl1') = 1" "dl1 queue and ownership reach $n"
done
retire core "$retire_sql" dl1
for n in core main star; do
  wait_for "scd_b1_$n" "SELECT (SELECT count(*) FROM public.sync_queue WHERE target_urn = 'urn:p:dl1') = 0 AND (SELECT count(*) FROM public.user_owned_playlists WHERE playlist_urn = 'urn:p:dl1') = 0" "dl1 queue and ownership are gone on $n"
done
want "core wrote the receipt" "$(only core "SELECT count(*) FROM public.repl_purges WHERE tbl = 'sync_queue' AND pk = '{\"id\": \"00000000-0000-0000-0000-0000000d0001\"}'")" "1"
want "core retired the playlist" "$(only core "SELECT deleted_at IS NOT NULL FROM public.playlists WHERE urn = 'urn:p:dl1'")" "t"

say "split deletes: the old direct delete leaves the row on the api node, which refuses it and queues a resend"
sql -d scd_b1_core -c "INSERT INTO public.playlists (urn, sc_playlist_id, title, title_normalized, owner_sc_user_id) VALUES ('urn:p:dl2', 'dl2', 'dl2', 'dl2', 'own-dl2')" > /dev/null
sql -d scd_b1_main -c "INSERT INTO public.sync_queue (id, user_id, action_type, target_urn) VALUES ('00000000-0000-0000-0000-0000000d0002', 'own-dl2', 'playlist_update', 'urn:p:dl2')" > /dev/null
for n in core main star; do
  wait_for "scd_b1_$n" "SELECT (SELECT count(*) FROM public.sync_queue WHERE target_urn = 'urn:p:dl2') = 1" "dl2 queue reaches $n"
done
retire core "$old_retire_sql" dl2
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.repl_resend WHERE tbl = 'sync_queue' AND pk = '{\"id\": \"00000000-0000-0000-0000-0000000d0002\"}')" "main refuses the receipt-less delete and queues a resend"
want "main still has the row core deleted" "$(queued_on main dl2)" "1"
want "core has no receipt for it" "$(only core "SELECT count(*) FROM public.repl_purges WHERE tbl = 'sync_queue' AND pk = '{\"id\": \"00000000-0000-0000-0000-0000000d0002\"}'")" "0"

say "split deletes: an admin oauth app delete from an api node runs on core and the row goes everywhere"
sql -d scd_b1_core -c "INSERT INTO public.oauth_apps (id, name, client_id, client_secret, redirect_uri) VALUES ('00000000-0000-0000-0000-0000000d0a01', 'dl', 'dl-client', 's', 'http://x')" > /dev/null
for n in main star; do
  wait_for "scd_b1_$n" "SELECT EXISTS (SELECT 1 FROM public.oauth_apps WHERE client_id = 'dl-client')" "the app reaches $n"
done
cid="$(sql -At -d scd_b1_main -c "SET scd.guard = 'enforce'" -c "SELECT id FROM public.scd_wt('oauth_app.delete', '{\"app_id\": \"00000000-0000-0000-0000-0000000d0a01\"}')")"
done_on_core "$cid" "the app delete"
for n in core main star; do
  wait_for "scd_b1_$n" "SELECT NOT EXISTS (SELECT 1 FROM public.oauth_apps WHERE client_id = 'dl-client')" "the app is gone on $n"
done
cid="$(sql -At -d scd_b1_main -c "SET scd.guard = 'enforce'" -c "SELECT id FROM public.scd_wt('oauth_app.delete', '{\"app_id\": \"00000000-0000-0000-0000-0000000d0a02\"}')")"
done_on_core "$cid" "the delete of a missing app"
want "no dead command" "$(only core "SELECT count(*) FROM public.cmd_dead WHERE kind = 'oauth_app.delete'")" "0"

say "split deletes: legacy mode runs both in place"
fresh scd_b1_legacy_main scd_b1_tpl
sql -d scd_b1_legacy_main -c "INSERT INTO public.playlists (urn, sc_playlist_id, title, title_normalized, owner_sc_user_id) VALUES ('urn:p:dl3', 'dl3', 'dl3', 'dl3', 'own-dl3')" \
  -c "INSERT INTO public.user_owned_playlists (user_id, playlist_urn) VALUES ('own-dl3', 'urn:p:dl3')" \
  -c "INSERT INTO public.sync_queue (id, user_id, action_type, target_urn) VALUES ('00000000-0000-0000-0000-0000000d0003', 'own-dl3', 'playlist_update', 'urn:p:dl3')" \
  -c "INSERT INTO public.oauth_apps (id, name, client_id, client_secret, redirect_uri) VALUES ('00000000-0000-0000-0000-0000000d0a03', 'dl', 'dl-legacy', 's', 'http://x')" > /dev/null
retire legacy_main "$retire_sql" dl3
want "legacy: queue and ownership are gone" "$(queued_on legacy_main dl3) $(owned_on legacy_main dl3)" "0 0"
sql -d scd_b1_legacy_main -c "SELECT id FROM public.scd_wt('oauth_app.delete', '{\"app_id\": \"00000000-0000-0000-0000-0000000d0a03\"}')" > /dev/null
want "legacy: the app is gone" "$(only legacy_main "SELECT count(*) FROM public.oauth_apps WHERE client_id = 'dl-legacy'")" "0"
sql -d postgres -c "DROP DATABASE scd_b1_legacy_main WITH (FORCE)" > /dev/null
