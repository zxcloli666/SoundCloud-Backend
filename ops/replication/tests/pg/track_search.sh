ts_drift="SELECT (
    SELECT count(*)
    FROM public.tracks AS track
    LEFT JOIN public.track_search AS search ON search.id = track.id
    WHERE track.sc_track_id LIKE 'ts%'
      AND track.sharing = 'public'
      AND track.deleted_at IS NULL
      AND track.superseded_by IS NULL
      AND (search.id IS NULL
           OR (track.sc_track_id, track.uploader_sc_user_id, track.title, track.title_normalized,
               track.metadata_artist, track.uploader_username, track.duration_ms, track.play_count_sc,
               public.track_search_variant(track.title_normalized))
              IS DISTINCT FROM
              (search.sc_track_id, search.uploader_sc_user_id, search.title, search.title_normalized,
               search.metadata_artist, search.uploader_username, search.duration_ms, search.play_count_sc,
               search.variant))
) + (
    SELECT count(*)
    FROM public.track_search AS search
    WHERE search.sc_track_id LIKE 'ts%'
      AND NOT EXISTS (
          SELECT 1
          FROM public.tracks AS track
          WHERE track.id = search.id
            AND track.sharing = 'public'
            AND track.deleted_at IS NULL
            AND track.superseded_by IS NULL
      )
)"

say "track_search: core derives the searchable tracks rows, main and star get them only by replication"
sql -d scd_b1_core -c "INSERT INTO public.tracks (id, urn, sc_track_id, title, title_normalized, duration_ms, sharing)
  SELECT gen_random_uuid(), 'urn:t:ts' || n, 'ts' || n, 'Ts ' || n,
         CASE WHEN n = 2 THEN 'ts 2 remix' ELSE 'ts ' || n END, 1000,
         CASE WHEN n = 3 THEN 'private' ELSE 'public' END
  FROM generate_series(1, 5) AS n" > /dev/null
sql -d scd_b1_core \
  -c "UPDATE public.tracks SET title = 'Ts one', title_normalized = 'ts one', play_count_sc = 9 WHERE sc_track_id = 'ts1'" \
  -c "UPDATE public.tracks SET deleted_at = now() WHERE sc_track_id = 'ts4'" \
  -c "UPDATE public.tracks SET superseded_by = (SELECT id FROM public.tracks WHERE sc_track_id = 'ts1') WHERE sc_track_id = 'ts5'" > /dev/null
calm
want "core: track_search equals the searchable tracks rows" "$(only core "$ts_drift")" 0
want "core: two searchable rows" "$(only core "SELECT string_agg(sc_track_id, ',' ORDER BY sc_track_id) FROM public.track_search WHERE sc_track_id LIKE 'ts%'")" ts1,ts2
want "core: the remix is a variant" "$(only core "SELECT variant FROM public.track_search WHERE sc_track_id = 'ts2'")" t
pairs track_search "sc_track_id LIKE 'ts%'" "track_search replicates from core"
pairs tracks "sc_track_id LIKE 'ts%'" "tracks replicate as before"
for n in main star; do
  want "$n: the replicated rows equal its tracks" "$(only "$n" "$ts_drift")" 0
done

say "track_search: a delete on core takes the row everywhere and brings back what it superseded"
sql -d scd_b1_core -c "DELETE FROM public.tracks WHERE sc_track_id = 'ts1'" > /dev/null
calm
want "core: ts5 is searchable again" "$(only core "SELECT string_agg(sc_track_id, ',' ORDER BY sc_track_id) FROM public.track_search WHERE sc_track_id LIKE 'ts%'")" ts2,ts5
want "core: track_search equals the searchable tracks rows" "$(only core "$ts_drift")" 0
pairs track_search "sc_track_id LIKE 'ts%'" "the delete and the comeback replicate"
