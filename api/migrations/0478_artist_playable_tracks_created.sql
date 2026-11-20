SET LOCAL lock_timeout = '5s';

ALTER TABLE public.artist_playable_tracks ADD COLUMN IF NOT EXISTS sc_created_at timestamptz;

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
    INSERT INTO public.artist_playable_tracks AS playable (artist_id, track_id, sc_track_id, ready_home, ready_wave, sc_created_at)
    SELECT ta.artist_id,
           NEW.id,
           NEW.sc_track_id,
           NEW.needs_duration_resolve IS NOT DISTINCT FROM false,
           NEW.index_state IS NOT DISTINCT FROM 'indexed',
           NEW.sc_created_at
    FROM public.track_artists ta
    WHERE ta.track_id = NEW.id
      AND ta.role = 'primary'
    ON CONFLICT (artist_id, track_id) DO UPDATE
    SET sc_track_id = EXCLUDED.sc_track_id,
        ready_home = EXCLUDED.ready_home,
        ready_wave = EXCLUDED.ready_wave,
        sc_created_at = EXCLUDED.sc_created_at
    WHERE (playable.sc_track_id, playable.ready_home, playable.ready_wave, playable.sc_created_at)
          IS DISTINCT FROM
          (EXCLUDED.sc_track_id, EXCLUDED.ready_home, EXCLUDED.ready_wave, EXCLUDED.sc_created_at);
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
        INSERT INTO public.artist_playable_tracks AS playable (artist_id, track_id, sc_track_id, ready_home, ready_wave, sc_created_at)
        SELECT NEW.artist_id,
               t.id,
               t.sc_track_id,
               t.needs_duration_resolve IS NOT DISTINCT FROM false,
               t.index_state IS NOT DISTINCT FROM 'indexed',
               t.sc_created_at
        FROM public.tracks t
        WHERE t.id = NEW.track_id
          AND t.sharing = 'public'
          AND t.superseded_by IS NULL
          AND t.storage_state = 'ok'
          AND (t.needs_duration_resolve = false OR t.index_state = 'indexed')
        ON CONFLICT (artist_id, track_id) DO UPDATE
        SET sc_track_id = EXCLUDED.sc_track_id,
            ready_home = EXCLUDED.ready_home,
            ready_wave = EXCLUDED.ready_wave,
            sc_created_at = EXCLUDED.sc_created_at
        WHERE (playable.sc_track_id, playable.ready_home, playable.ready_wave, playable.sc_created_at)
              IS DISTINCT FROM
              (EXCLUDED.sc_track_id, EXCLUDED.ready_home, EXCLUDED.ready_wave, EXCLUDED.sc_created_at);
    END IF;
    RETURN NULL;
END
$$;

CREATE OR REPLACE TRIGGER artist_playable_tracks_track_update
    AFTER UPDATE OF id, sc_track_id, sharing, superseded_by, storage_state, needs_duration_resolve, index_state, sc_created_at
    ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.id IS DISTINCT FROM NEW.id
           OR OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id
           OR OLD.sharing IS DISTINCT FROM NEW.sharing
           OR OLD.superseded_by IS DISTINCT FROM NEW.superseded_by
           OR OLD.storage_state IS DISTINCT FROM NEW.storage_state
           OR OLD.needs_duration_resolve IS DISTINCT FROM NEW.needs_duration_resolve
           OR OLD.index_state IS DISTINCT FROM NEW.index_state
           OR OLD.sc_created_at IS DISTINCT FROM NEW.sc_created_at))
    EXECUTE FUNCTION public.artist_playable_tracks_sync_track();

ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER artist_playable_tracks_track_update;
