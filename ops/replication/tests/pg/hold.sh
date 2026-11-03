say "B5b hold: test kinds for projection rows and owned rows (same body on every node)"
for n in $nodes; do
  sql -d "scd_b1_$n" -f - <<'SQL'
CREATE OR REPLACE FUNCTION public.scd_cmd_playlist_membership_op(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
BEGIN
    DELETE FROM public.playlist_track_projection p
    WHERE p.playlist_urn = args ->> 'urn' AND p.position IN (SELECT (d.value)::integer FROM jsonb_array_elements_text(coalesce(args -> 'drop', '[]')) d);
    INSERT INTO public.playlist_track_projection (playlist_urn, position, sc_track_id)
    SELECT args ->> 'urn', (s.value ->> 'position')::integer, s.value ->> 'track' FROM jsonb_array_elements(coalesce(args -> 'set', '[]')) s
    ON CONFLICT (playlist_urn, position) DO UPDATE SET sc_track_id = EXCLUDED.sc_track_id;
    RETURN jsonb_build_object('touched', (
        SELECT coalesce(jsonb_agg(jsonb_build_object('tbl', 'playlist_track_projection', 'pk', jsonb_build_object('playlist_urn', args ->> 'urn', 'position', x.position))), '[]')
        FROM (SELECT (d.value)::integer AS position FROM jsonb_array_elements_text(coalesce(args -> 'drop', '[]')) d
              UNION SELECT (s.value ->> 'position')::integer FROM jsonb_array_elements(coalesce(args -> 'set', '[]')) s) x));
END
$f$;
CREATE OR REPLACE FUNCTION public.scd_cmd_owned_insert(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
BEGIN
    IF (args ->> 'update_only')::boolean THEN
        UPDATE public.user_owned_tracks SET progress = (args ->> 'progress')::boolean
        WHERE user_id = args ->> 'user' AND sc_track_id = args ->> 'track';
    ELSE
        INSERT INTO public.user_owned_tracks (user_id, sc_track_id, progress, created_at)
        VALUES (args ->> 'user', args ->> 'track', (args ->> 'progress')::boolean, (args ->> 'at')::timestamptz)
        ON CONFLICT (user_id, sc_track_id) DO UPDATE SET progress = EXCLUDED.progress;
    END IF;
    RETURN jsonb_build_object('touched', jsonb_build_array(jsonb_build_object('tbl', 'user_owned_tracks',
        'pk', jsonb_build_object('user_id', args ->> 'user', 'sc_track_id', args ->> 'track'))));
END
$f$;
SQL
done
echo "ok hold kinds installed"

hold_mk=$((0xe5b0))
seen_on() {
  hold_mk=$((hold_mk + 1))
  local id
  id="$(printf '%04x' "$hold_mk")"
  mark core "$id"
  wait_for "scd_b1_$1" "SELECT EXISTS (SELECT 1 FROM public.user_events WHERE id = '00000000-0000-0000-0000-00000000$id')" "$1 applied core's stream up to here"
}
proj() { only "$1" "SELECT coalesce(string_agg(position || ':' || sc_track_id, ' ' ORDER BY position), '-') FROM public.playlist_track_projection WHERE playlist_urn = '$2'"; }
owned() { only "$1" "SELECT coalesce(string_agg(sc_track_id || ' ' || progress, '|' ORDER BY sc_track_id), '-') FROM public.user_owned_tracks WHERE user_id = '$2'"; }
done_on_core() { wait_for scd_b1_core "SELECT EXISTS (SELECT 1 FROM public.cmd_done WHERE id = '$1')" "core ran command $2"; }

say "B5b hold: a pending key keeps its kind-time row through core's delete and reinsert of the same key"
sql -d scd_b1_core -c "INSERT INTO public.playlists (urn, sc_playlist_id, title, title_normalized) VALUES ('urn:p:h1', 'h1', 'H1', 'h1')" \
  -c "INSERT INTO public.playlist_track_projection (playlist_urn, position, sc_track_id) VALUES ('urn:p:h1', 0, 'a'), ('urn:p:h1', 1, 'b')"
wait_for scd_b1_main "SELECT count(*) = 2 FROM public.playlist_track_projection WHERE playlist_urn = 'urn:p:h1'" "h1 reaches main"
wait_for scd_b1_star "SELECT count(*) = 2 FROM public.playlist_track_projection WHERE playlist_urn = 'urn:p:h1'" "h1 reaches star"
pause core core_from_main
cid="$(only main "SELECT id FROM public.scd_wt('playlist.membership_op', '{\"urn\": \"urn:p:h1\", \"set\": [{\"position\": 0, \"track\": \"c\"}]}')")"
want "main's optimistic row" "$(proj main urn:p:h1)" "0:c 1:b"
sql -d scd_b1_core -c "BEGIN" -c "DELETE FROM public.playlist_track_projection WHERE playlist_urn = 'urn:p:h1'" \
  -c "INSERT INTO public.playlist_track_projection (playlist_urn, position, sc_track_id) VALUES ('urn:p:h1', 0, 'a'), ('urn:p:h1', 1, 'b'), ('urn:p:h1', 2, 'd')" -c "COMMIT"
seen_on main
want "main holds the pending key through the delete and the insert, the other keys follow core" "$(proj main urn:p:h1)" "0:c 1:b 2:d"
resume core core_from_main
done_on_core "$cid" "the h1 edit"
calm
want "core ran the edit on its own rows" "$(proj core urn:p:h1)" "0:c 1:b 2:d"
pairs playlist_track_projection "playlist_urn = 'urn:p:h1'" "h1 equal on all nodes"

say "B5b hold: a PK move onto a pending key and off a pending key"
sql -d scd_b1_core -c "INSERT INTO public.user_owned_tracks (user_id, sc_track_id, progress, created_at) VALUES ('mv', 's1', false, '2026-01-01'), ('mv', 's2', false, '2026-01-01')"
wait_for scd_b1_main "SELECT count(*) = 2 FROM public.user_owned_tracks WHERE user_id = 'mv'" "owned rows reach main"
wait_for scd_b1_star "SELECT count(*) = 2 FROM public.user_owned_tracks WHERE user_id = 'mv'" "owned rows reach star"
pause core core_from_main
cid1="$(only main "SELECT id FROM public.scd_wt('owned.insert', '{\"user\": \"mv\", \"track\": \"s3\", \"progress\": true, \"at\": \"2026-02-01T00:00:00Z\"}')")"
cid2="$(only main "SELECT id FROM public.scd_wt('owned.insert', '{\"user\": \"mv\", \"track\": \"s1\", \"progress\": true, \"update_only\": true}')")"
want "main's optimistic rows" "$(owned main mv)" "s1 true|s2 false|s3 true"
sql -d scd_b1_core -c "UPDATE public.user_owned_tracks SET sc_track_id = 's3' WHERE user_id = 'mv' AND sc_track_id = 's2'" \
  -c "UPDATE public.user_owned_tracks SET sc_track_id = 's4' WHERE user_id = 'mv' AND sc_track_id = 's1'"
seen_on main
want "main drops the row that moved onto pending s3, keeps pending s1 and takes the row that moved off it as s4" "$(owned main mv)" "s1 true|s3 true|s4 false"
resume core core_from_main
done_on_core "$cid1" "the s3 insert"
done_on_core "$cid2" "the s1 update"
calm
want "core: the moved row took the insert, s1 is gone" "$(owned core mv)" "s3 true|s4 false"
want "core purged s1 on main" "$(only core "SELECT count(*) FROM public.cmd_purge WHERE id = '$cid2'")" 1
pairs user_owned_tracks "user_id = 'mv'" "owned rows equal on all nodes"

say "B5b hold: a unique collision purges the held row and the unconditional reassert brings it back"
sql -d scd_b1_core -c "INSERT INTO public.playlists (urn, sc_playlist_id, title, title_normalized) VALUES ('urn:p:h2', 'h2', 'H2', 'h2')"
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.playlists WHERE urn = 'urn:p:h2')" "h2 reaches main"
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM public.playlists WHERE urn = 'urn:p:h2')" "h2 reaches star"
pause core core_from_main
cid="$(only main "SELECT id FROM public.scd_wt('playlist.membership_op', '{\"urn\": \"urn:p:h2\", \"set\": [{\"position\": 0, \"track\": \"x\"}]}')")"
want "main's optimistic row" "$(proj main urn:p:h2)" "0:x"
sql -d scd_b1_core -c "INSERT INTO public.playlist_track_projection (playlist_urn, position, sc_track_id) VALUES ('urn:p:h2', 1, 'x')"
seen_on main
want "core's row at an untouched key wins the unique key over the held row" "$(proj main urn:p:h2)" "1:x"
sql -d scd_b1_core -c "BEGIN" -c "DELETE FROM public.playlist_track_projection WHERE playlist_urn = 'urn:p:h2'" \
  -c "INSERT INTO public.playlist_track_projection (playlist_urn, position, sc_track_id) VALUES ('urn:p:h2', 0, 'x')" -c "COMMIT"
seen_on main
want "core's row at the pending key is held, so main has none" "$(proj main urn:p:h2)" "-"
resume core core_from_main
done_on_core "$cid" "the h2 edit"
calm
want "core's row equals main's kind-time row" "$(proj core urn:p:h2)" "0:x"
want "core reasserts it although the hash is equal" "$(only core "SELECT count(*) FROM public.cmd_reassert WHERE id = '$cid' AND tbl = 'playlist_track_projection'")" 1
want "main has it back" "$(proj main urn:p:h2)" "0:x"
pairs playlist_track_projection "playlist_urn = 'urn:p:h2'" "h2 equal on all nodes"

say "B5b hold: core deletes a pending key for good"
sql -d scd_b1_core -c "INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms) VALUES ('00000000-0000-0000-0000-0000000e5b04', 'urn:t:h4', 'h4', 'Old', 'old', 1000)"
wait_for scd_b1_main "SELECT EXISTS (SELECT 1 FROM public.tracks WHERE urn = 'urn:t:h4')" "h4 reaches main"
wait_for scd_b1_star "SELECT EXISTS (SELECT 1 FROM public.tracks WHERE urn = 'urn:t:h4')" "h4 reaches star"
pause core core_from_main
cid="$(only main "SELECT id FROM public.scd_wt('track.edit', '{\"urn\": \"urn:t:h4\", \"title\": \"Mine\"}')")"
sql -d scd_b1_core -c "DELETE FROM public.tracks WHERE urn = 'urn:t:h4'"
seen_on main
want "main holds the delete of the pending track" "$(only main "SELECT title FROM public.tracks WHERE urn = 'urn:t:h4'")" "Mine"
seen_on star
want "star applies it" "$(only star "SELECT count(*) FROM public.tracks WHERE urn = 'urn:t:h4'")" 0
resume core core_from_main
done_on_core "$cid" "the h4 edit"
calm
want "core purges the key on main" "$(only core "SELECT count(*) FROM public.cmd_purge WHERE id = '$cid'")" 1
want "main drops the track" "$(only main "SELECT count(*) FROM public.tracks WHERE urn = 'urn:t:h4'")" 0
pairs tracks "urn = 'urn:t:h4'" "h4 gone on all nodes"

say "B5b hold: a key unique stays with core's row, the held row is purged and the dispatch purges its key"
pause core core_from_main
cid="$(only main "SELECT id FROM public.scd_wt('observe.playlists', '{\"urn\": \"urn:p:k1\", \"sc_id\": \"pk1\", \"title\": \"K1\"}')")"
sql -d scd_b1_core -c "INSERT INTO public.playlists (urn, sc_playlist_id, title, title_normalized) VALUES ('urn:p:k2', 'pk1', 'K2', 'k2')"
seen_on main
want "main takes core's row and purges the held one with its membership row" \
  "$(only main "SELECT string_agg(urn, ',') || ' ' || (SELECT count(*) FROM public.playlist_membership_state WHERE playlist_urn = 'urn:p:k1') FROM public.playlists WHERE sc_playlist_id = 'pk1' OR urn = 'urn:p:k1'")" "urn:p:k2 0"
resume core core_from_main
done_on_core "$cid" "the k1 observe"
calm
want "core refuses the kind on the taken key" "$(only core "SELECT status FROM public.cmd_done WHERE id = '$cid'")" dead
want "and purges both touched keys" "$(only core "SELECT count(*) FROM public.cmd_purge WHERE id = '$cid'")" 2
pairs playlists "sc_playlist_id = 'pk1' OR urn = 'urn:p:k1'" "pk1 on one key on all nodes"
pairs playlist_membership_state "playlist_urn IN ('urn:p:k1', 'urn:p:k2')" "membership rows equal on all nodes"

say "B5b least: one user_events id born on three nodes ends with the earliest created_at everywhere"
hub_subs="core:core_from_main core:core_from_star main:main_from_core star:star_from_core"
for s in $hub_subs; do pause "${s%%:*}" "${s##*:}"; done
ue="00000000-0000-0000-0000-0000000e5b10"
sql -d scd_b1_main -c "INSERT INTO public.user_events (id, sc_user_id, sc_track_id, event_type, weight, created_at) VALUES ('$ue', 'le', 't', 'playlist_add', 1, '2026-03-01')"
sql -d scd_b1_star -c "INSERT INTO public.user_events (id, sc_user_id, sc_track_id, event_type, weight, created_at) VALUES ('$ue', 'le', 't', 'playlist_add', 1, '2026-03-02')"
sql -d scd_b1_core -c "INSERT INTO public.user_events (id, sc_user_id, sc_track_id, event_type, weight, created_at) VALUES ('$ue', 'le', 't', 'playlist_add', 1, '2026-03-03')"
born() { only "$1" "SELECT created_at::date FROM public.user_events WHERE id = '$ue'"; }
resume core core_from_star
wait_for scd_b1_core "SELECT created_at::date = '2026-03-02' FROM public.user_events WHERE id = '$ue'" "core merges star's earlier birth down"
resume main main_from_core
seen_on main
want "main keeps its earlier birth against core's later update" "$(born main)" 2026-03-01
resume core core_from_main
resume star star_from_core
for n in $nodes; do
  wait_for "scd_b1_$n" "SELECT bool_and(pid IS NOT NULL) FROM pg_catalog.pg_stat_subscription WHERE relid IS NULL AND leader_pid IS NULL" "apply workers up again on $n"
done
calm
want "core" "$(born core)" 2026-03-01
want "star" "$(born star)" 2026-03-01
pairs user_events "id = '$ue'" "the event equal on all nodes"

for t in playlist_track_projection user_owned_tracks playlists playlist_membership_state tracks; do
  pairs "$t" true "whole table $t after the hold cases"
done
