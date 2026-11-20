SET LOCAL lock_timeout = '5s';

ALTER TABLE public.admin_maintenance_runs
    DROP CONSTRAINT admin_maintenance_runs_kind_valid,
    ADD CONSTRAINT admin_maintenance_runs_kind_valid CHECK (
        kind IN ('catalog_renormalize', 'musicbrainz_names', 'wanted_aliases', 'lyrics_lookup_queue', 'track_search', 'lyrics_refetch', 'lyrics_description', 'track_sync_stamps', 'track_work_keys', 'track_enrich_queue', 'artist_playable_tracks', 'artist_track_titles')
    );

CREATE OR REPLACE FUNCTION public.artist_track_titles_sync_track()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $$
BEGIN
    IF TG_OP = 'DELETE' OR (TG_OP = 'UPDATE' AND NEW.id IS DISTINCT FROM OLD.id) THEN
        DELETE FROM public.artist_track_titles WHERE track_id = OLD.id;
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN NULL;
    END IF;
    IF NEW.superseded_by IS NOT NULL OR NEW.title_normalized IS NULL THEN
        DELETE FROM public.artist_track_titles WHERE track_id = NEW.id;
        RETURN NULL;
    END IF;
    DELETE FROM public.artist_track_titles stored
    WHERE stored.track_id = NEW.id
      AND (stored.title_normalized <> NEW.title_normalized COLLATE "C"
           OR (stored.artist_id IS DISTINCT FROM NEW.primary_artist_id
               AND NOT EXISTS (
                   SELECT 1
                   FROM public.track_artists ta
                   WHERE ta.track_id = NEW.id
                     AND ta.artist_id = stored.artist_id
                     AND ta.role = 'primary'
               )));
    INSERT INTO public.artist_track_titles (artist_id, title_normalized, track_id)
    SELECT wanted.artist_id, NEW.title_normalized, NEW.id
    FROM (
        SELECT NEW.primary_artist_id AS artist_id
        WHERE NEW.primary_artist_id IS NOT NULL
        UNION
        SELECT ta.artist_id
        FROM public.track_artists ta
        WHERE ta.track_id = NEW.id
          AND ta.role = 'primary'
    ) AS wanted
    ORDER BY wanted.artist_id
    ON CONFLICT DO NOTHING;
    RETURN NULL;
END
$$;

CREATE OR REPLACE FUNCTION public.artist_track_titles_sync_artist()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $$
BEGIN
    IF TG_OP IN ('DELETE', 'UPDATE') AND OLD.role = 'primary' THEN
        DELETE FROM public.artist_track_titles stored
        WHERE stored.track_id = OLD.track_id
          AND stored.artist_id = OLD.artist_id
          AND NOT EXISTS (
              SELECT 1
              FROM public.tracks t
              WHERE t.id = OLD.track_id
                AND t.primary_artist_id = OLD.artist_id
          );
    END IF;
    IF TG_OP IN ('INSERT', 'UPDATE') AND NEW.role = 'primary' THEN
        INSERT INTO public.artist_track_titles (artist_id, title_normalized, track_id)
        SELECT NEW.artist_id, t.title_normalized, t.id
        FROM public.tracks t
        WHERE t.id = NEW.track_id
          AND t.superseded_by IS NULL
          AND t.title_normalized IS NOT NULL
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NULL;
END
$$;

CREATE OR REPLACE TRIGGER artist_track_titles_track_update
    AFTER UPDATE OF id, title_normalized, primary_artist_id, superseded_by
    ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.id IS DISTINCT FROM NEW.id
           OR OLD.title_normalized IS DISTINCT FROM NEW.title_normalized
           OR OLD.primary_artist_id IS DISTINCT FROM NEW.primary_artist_id
           OR (OLD.superseded_by IS NULL) IS DISTINCT FROM (NEW.superseded_by IS NULL)))
    EXECUTE FUNCTION public.artist_track_titles_sync_track();

CREATE OR REPLACE TRIGGER artist_track_titles_track_row
    AFTER INSERT OR DELETE ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.artist_track_titles_sync_track();

CREATE OR REPLACE TRIGGER artist_track_titles_artist_update
    AFTER UPDATE OF track_id, artist_id, role
    ON public.track_artists
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.track_id IS DISTINCT FROM NEW.track_id
           OR OLD.artist_id IS DISTINCT FROM NEW.artist_id
           OR OLD.role IS DISTINCT FROM NEW.role))
    EXECUTE FUNCTION public.artist_track_titles_sync_artist();

CREATE OR REPLACE TRIGGER artist_track_titles_artist_row
    AFTER INSERT OR DELETE ON public.track_artists
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.artist_track_titles_sync_artist();

ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER artist_track_titles_track_update;

ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER artist_track_titles_track_row;

ALTER TABLE public.track_artists ENABLE ALWAYS TRIGGER artist_track_titles_artist_update;

ALTER TABLE public.track_artists ENABLE ALWAYS TRIGGER artist_track_titles_artist_row;
