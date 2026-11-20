SET LOCAL lock_timeout = '5s';

ALTER TABLE public.admin_maintenance_runs
    DROP CONSTRAINT admin_maintenance_runs_kind_valid,
    ADD CONSTRAINT admin_maintenance_runs_kind_valid CHECK (
        kind IN ('catalog_renormalize', 'musicbrainz_names', 'wanted_aliases', 'lyrics_lookup_queue', 'track_search', 'lyrics_refetch', 'lyrics_description', 'track_sync_stamps', 'track_work_keys', 'track_enrich_queue', 'artist_playable_tracks', 'artist_track_titles', 'lyrics_fts_inline')
    );

ALTER TABLE public.lyrics_cache ALTER COLUMN fts SET STORAGE MAIN;
