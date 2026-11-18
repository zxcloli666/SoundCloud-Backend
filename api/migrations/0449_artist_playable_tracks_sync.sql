SET LOCAL lock_timeout = '5s';

ALTER TABLE public.admin_maintenance_runs
    DROP CONSTRAINT admin_maintenance_runs_kind_valid,
    ADD CONSTRAINT admin_maintenance_runs_kind_valid CHECK (
        kind IN ('catalog_renormalize', 'musicbrainz_names', 'wanted_aliases', 'lyrics_lookup_queue', 'track_search', 'lyrics_refetch', 'lyrics_description', 'track_sync_stamps', 'track_work_keys', 'track_enrich_queue', 'artist_playable_tracks')
    );

CREATE OR REPLACE FUNCTION public.artist_playable_tracks_sync_track()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        DELETE FROM public.artist_playable_tracks WHERE track_id = OLD.id;
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' AND NEW.id IS DISTINCT FROM OLD.id THEN
        DELETE FROM public.artist_playable_tracks WHERE track_id = OLD.id;
    END IF;
    IF NEW.sharing IS DISTINCT FROM 'public'
       OR NEW.superseded_by IS NOT NULL
       OR NEW.storage_state IS DISTINCT FROM 'ok'
       OR (NEW.needs_duration_resolve IS DISTINCT FROM false AND NEW.index_state IS DISTINCT FROM 'indexed') THEN
        DELETE FROM public.artist_playable_tracks WHERE track_id = NEW.id;
        RETURN NULL;
    END IF;
    INSERT INTO public.artist_playable_tracks AS playable (artist_id, track_id, sc_track_id, ready_home, ready_wave)
    SELECT ta.artist_id,
           NEW.id,
           NEW.sc_track_id,
           NEW.needs_duration_resolve IS NOT DISTINCT FROM false,
           NEW.index_state IS NOT DISTINCT FROM 'indexed'
    FROM public.track_artists ta
    WHERE ta.track_id = NEW.id
      AND ta.role = 'primary'
    ON CONFLICT (artist_id, track_id) DO UPDATE
    SET sc_track_id = EXCLUDED.sc_track_id,
        ready_home = EXCLUDED.ready_home,
        ready_wave = EXCLUDED.ready_wave
    WHERE (playable.sc_track_id, playable.ready_home, playable.ready_wave)
          IS DISTINCT FROM
          (EXCLUDED.sc_track_id, EXCLUDED.ready_home, EXCLUDED.ready_wave);
    RETURN NULL;
END
$$;

CREATE OR REPLACE FUNCTION public.artist_playable_tracks_sync_artist()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $$
BEGIN
    IF TG_OP IN ('DELETE', 'UPDATE') AND OLD.role = 'primary' THEN
        DELETE FROM public.artist_playable_tracks
        WHERE artist_id = OLD.artist_id
          AND track_id = OLD.track_id;
    END IF;
    IF TG_OP IN ('INSERT', 'UPDATE') AND NEW.role = 'primary' THEN
        INSERT INTO public.artist_playable_tracks AS playable (artist_id, track_id, sc_track_id, ready_home, ready_wave)
        SELECT NEW.artist_id,
               t.id,
               t.sc_track_id,
               t.needs_duration_resolve IS NOT DISTINCT FROM false,
               t.index_state IS NOT DISTINCT FROM 'indexed'
        FROM public.tracks t
        WHERE t.id = NEW.track_id
          AND t.sharing = 'public'
          AND t.superseded_by IS NULL
          AND t.storage_state = 'ok'
          AND (t.needs_duration_resolve = false OR t.index_state = 'indexed')
        ON CONFLICT (artist_id, track_id) DO UPDATE
        SET sc_track_id = EXCLUDED.sc_track_id,
            ready_home = EXCLUDED.ready_home,
            ready_wave = EXCLUDED.ready_wave
        WHERE (playable.sc_track_id, playable.ready_home, playable.ready_wave)
              IS DISTINCT FROM
              (EXCLUDED.sc_track_id, EXCLUDED.ready_home, EXCLUDED.ready_wave);
    END IF;
    RETURN NULL;
END
$$;

CREATE OR REPLACE TRIGGER artist_playable_tracks_track_update
    AFTER UPDATE OF id, sc_track_id, sharing, superseded_by, storage_state, needs_duration_resolve, index_state
    ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.id IS DISTINCT FROM NEW.id
           OR OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id
           OR OLD.sharing IS DISTINCT FROM NEW.sharing
           OR OLD.superseded_by IS DISTINCT FROM NEW.superseded_by
           OR OLD.storage_state IS DISTINCT FROM NEW.storage_state
           OR OLD.needs_duration_resolve IS DISTINCT FROM NEW.needs_duration_resolve
           OR OLD.index_state IS DISTINCT FROM NEW.index_state))
    EXECUTE FUNCTION public.artist_playable_tracks_sync_track();

CREATE OR REPLACE TRIGGER artist_playable_tracks_track_row
    AFTER INSERT OR DELETE ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.artist_playable_tracks_sync_track();

CREATE OR REPLACE TRIGGER artist_playable_tracks_artist_update
    AFTER UPDATE OF track_id, artist_id, role
    ON public.track_artists
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.track_id IS DISTINCT FROM NEW.track_id
           OR OLD.artist_id IS DISTINCT FROM NEW.artist_id
           OR OLD.role IS DISTINCT FROM NEW.role))
    EXECUTE FUNCTION public.artist_playable_tracks_sync_artist();

CREATE OR REPLACE TRIGGER artist_playable_tracks_artist_row
    AFTER INSERT OR DELETE ON public.track_artists
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.artist_playable_tracks_sync_artist();

ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER artist_playable_tracks_track_update;

ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER artist_playable_tracks_track_row;

ALTER TABLE public.track_artists ENABLE ALWAYS TRIGGER artist_playable_tracks_artist_update;

ALTER TABLE public.track_artists ENABLE ALWAYS TRIGGER artist_playable_tracks_artist_row;
