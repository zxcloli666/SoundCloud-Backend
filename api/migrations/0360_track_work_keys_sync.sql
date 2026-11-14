SET LOCAL lock_timeout = '5s';

ALTER TABLE public.admin_maintenance_runs
    DROP CONSTRAINT admin_maintenance_runs_kind_valid,
    ADD CONSTRAINT admin_maintenance_runs_kind_valid CHECK (
        kind IN ('catalog_renormalize', 'musicbrainz_names', 'wanted_aliases', 'lyrics_lookup_queue', 'track_search', 'lyrics_refetch', 'lyrics_description', 'track_sync_stamps', 'track_work_keys')
    );

CREATE OR REPLACE FUNCTION public.track_work_keys_sync()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $$
BEGIN
    INSERT INTO public.track_work_keys AS keys (track_id, primary_artist_id, superseded)
    VALUES (NEW.id, NEW.primary_artist_id, NEW.superseded_by IS NOT NULL)
    ON CONFLICT (track_id) DO UPDATE
    SET primary_artist_id = EXCLUDED.primary_artist_id,
        superseded = EXCLUDED.superseded
    WHERE (keys.primary_artist_id, keys.superseded)
          IS DISTINCT FROM (EXCLUDED.primary_artist_id, EXCLUDED.superseded);
    RETURN NULL;
END
$$;

CREATE OR REPLACE TRIGGER track_work_keys_update
    AFTER UPDATE OF primary_artist_id, superseded_by
    ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.primary_artist_id IS DISTINCT FROM NEW.primary_artist_id
           OR (OLD.superseded_by IS NULL) IS DISTINCT FROM (NEW.superseded_by IS NULL)))
    EXECUTE FUNCTION public.track_work_keys_sync();

CREATE OR REPLACE TRIGGER track_work_keys_row
    AFTER INSERT ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.track_work_keys_sync();

ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER track_work_keys_update;
ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER track_work_keys_row;
