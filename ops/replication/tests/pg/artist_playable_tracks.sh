ap_drift="SELECT (
    SELECT count(*)
    FROM public.tracks AS track
    JOIN public.track_artists AS credit ON credit.track_id = track.id AND credit.role = 'primary'
    LEFT JOIN public.artist_playable_tracks AS playable
           ON playable.artist_id = credit.artist_id AND playable.track_id = track.id
    WHERE track.sc_track_id LIKE 'ap%'
      AND track.sharing = 'public'
      AND track.superseded_by IS NULL
      AND track.storage_state = 'ok'
      AND (track.needs_duration_resolve = false OR track.index_state = 'indexed')
      AND (playable.track_id IS NULL
           OR (track.sc_track_id, track.needs_duration_resolve = false, track.index_state = 'indexed')
              IS DISTINCT FROM
              (playable.sc_track_id, playable.ready_home, playable.ready_wave))
) + (
    SELECT count(*)
    FROM public.artist_playable_tracks AS playable
    WHERE playable.sc_track_id LIKE 'ap%'
      AND NOT EXISTS (
          SELECT 1
          FROM public.tracks AS track
          JOIN public.track_artists AS credit ON credit.track_id = track.id AND credit.role = 'primary'
          WHERE track.id = playable.track_id
            AND credit.artist_id = playable.artist_id
            AND track.sharing = 'public'
            AND track.superseded_by IS NULL
            AND track.storage_state = 'ok'
            AND (track.needs_duration_resolve = false OR track.index_state = 'indexed')
      )
)"

say "artist_playable_tracks: core derives the playable primary rows, main and star get them only by replication"
sql -d scd_b1_core -c "INSERT INTO public.artists (id, name, normalized_name, source)
  SELECT md5('ap artist ' || n)::uuid, 'Ap ' || n, 'ap ' || n, 'sc' FROM generate_series(1, 2) AS n" > /dev/null
sql -d scd_b1_core -c "INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms, sharing, storage_state, needs_duration_resolve, index_state)
  SELECT md5('ap track ' || n)::uuid, 'urn:t:ap' || n, 'ap' || n, 'Ap ' || n, 'ap ' || n, 1000,
         CASE WHEN n = 3 THEN 'private' ELSE 'public' END,
         CASE WHEN n = 4 THEN 'pending' ELSE 'ok' END,
         n = 5,
         CASE WHEN n IN (5, 6) THEN 'indexed' ELSE 'pending' END
  FROM generate_series(1, 6) AS n" > /dev/null
sql -d scd_b1_core -c "INSERT INTO public.track_artists (track_id, artist_id, role, source)
  SELECT md5('ap track ' || n)::uuid, md5('ap artist ' || (1 + n % 2))::uuid, 'primary', 'sc' FROM generate_series(1, 6) AS n" \
  -c "INSERT INTO public.track_artists (track_id, artist_id, role, source)
  SELECT md5('ap track 1')::uuid, md5('ap artist 1')::uuid, 'featured', 'sc'" > /dev/null
calm
ap_ids="SELECT string_agg(sc_track_id || ':' || ready_home || ':' || ready_wave, ',' ORDER BY sc_track_id) FROM public.artist_playable_tracks WHERE sc_track_id LIKE 'ap%'"
want "core: artist_playable_tracks equals the playable primary rows" "$(only core "$ap_drift")" 0
want "core: the playable rows" "$(only core "$ap_ids")" ap1:true:false,ap2:true:false,ap5:false:true,ap6:true:true
pairs artist_playable_tracks "sc_track_id LIKE 'ap%'" "artist_playable_tracks replicates from core"
for n in main star; do
  want "$n: the replicated rows equal its tracks" "$(only "$n" "$ap_drift")" 0
done

say "artist_playable_tracks: state changes and credit changes follow on core"
sql -d scd_b1_core \
  -c "UPDATE public.tracks SET storage_state = 'ok' WHERE sc_track_id = 'ap4'" \
  -c "UPDATE public.tracks SET sharing = 'private' WHERE sc_track_id = 'ap1'" \
  -c "UPDATE public.tracks SET needs_duration_resolve = true, index_state = 'pending' WHERE sc_track_id = 'ap2'" \
  -c "UPDATE public.tracks SET index_state = 'pending', needs_duration_resolve = false WHERE sc_track_id = 'ap6'" \
  -c "UPDATE public.track_artists SET role = 'featured' WHERE track_id = md5('ap track 5')::uuid" > /dev/null
calm
want "core: artist_playable_tracks equals the playable primary rows" "$(only core "$ap_drift")" 0
want "core: the playable rows after the changes" "$(only core "$ap_ids")" ap4:true:false,ap6:true:false
pairs artist_playable_tracks "sc_track_id LIKE 'ap%'" "the state and credit changes replicate"

say "artist_playable_tracks: a delete on core takes the rows everywhere"
sql -d scd_b1_core -c "DELETE FROM public.tracks WHERE sc_track_id = 'ap4'" > /dev/null
calm
want "core: the playable rows after the delete" "$(only core "$ap_ids")" ap6:true:false
want "core: artist_playable_tracks equals the playable primary rows" "$(only core "$ap_drift")" 0
pairs artist_playable_tracks "sc_track_id LIKE 'ap%'" "the delete replicates"
