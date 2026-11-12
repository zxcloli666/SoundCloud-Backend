SET LOCAL lock_timeout = '5s';

ALTER TABLE public.admin_maintenance_runs
    DROP CONSTRAINT admin_maintenance_runs_kind_valid,
    ADD CONSTRAINT admin_maintenance_runs_kind_valid CHECK (
        kind IN ('catalog_renormalize', 'musicbrainz_names', 'wanted_aliases', 'lyrics_lookup_queue', 'track_search')
    );

CREATE OR REPLACE FUNCTION public.track_search_variant(title_normalized text)
RETURNS boolean
LANGUAGE sql
IMMUTABLE
PARALLEL SAFE
AS $$
SELECT title_normalized ~ '\m(remix|sped up|slowed|nightcore|cover|live|8d|reverb|bass boosted|instrumental|karaoke|mashup|edit)\M'
$$;

CREATE OR REPLACE FUNCTION public.track_search_sync()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        DELETE FROM public.track_search WHERE id = OLD.id;
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' AND NEW.id IS DISTINCT FROM OLD.id THEN
        DELETE FROM public.track_search WHERE id = OLD.id;
    END IF;
    IF NEW.sharing IS DISTINCT FROM 'public' OR NEW.deleted_at IS NOT NULL OR NEW.superseded_by IS NOT NULL THEN
        DELETE FROM public.track_search WHERE id = NEW.id;
        RETURN NULL;
    END IF;
    INSERT INTO public.track_search AS search (
        id, sc_track_id, uploader_sc_user_id, title, title_normalized,
        metadata_artist, uploader_username, duration_ms, play_count_sc, variant
    )
    VALUES (
        NEW.id, NEW.sc_track_id, NEW.uploader_sc_user_id, NEW.title, NEW.title_normalized,
        NEW.metadata_artist, NEW.uploader_username, NEW.duration_ms, NEW.play_count_sc,
        public.track_search_variant(NEW.title_normalized)
    )
    ON CONFLICT (id) DO UPDATE
    SET sc_track_id = EXCLUDED.sc_track_id,
        uploader_sc_user_id = EXCLUDED.uploader_sc_user_id,
        title = EXCLUDED.title,
        title_normalized = EXCLUDED.title_normalized,
        metadata_artist = EXCLUDED.metadata_artist,
        uploader_username = EXCLUDED.uploader_username,
        duration_ms = EXCLUDED.duration_ms,
        play_count_sc = EXCLUDED.play_count_sc,
        variant = EXCLUDED.variant
    WHERE (search.sc_track_id, search.uploader_sc_user_id, search.title, search.title_normalized,
           search.metadata_artist, search.uploader_username, search.duration_ms, search.play_count_sc,
           search.variant)
          IS DISTINCT FROM
          (EXCLUDED.sc_track_id, EXCLUDED.uploader_sc_user_id, EXCLUDED.title, EXCLUDED.title_normalized,
           EXCLUDED.metadata_artist, EXCLUDED.uploader_username, EXCLUDED.duration_ms, EXCLUDED.play_count_sc,
           EXCLUDED.variant);
    RETURN NULL;
END
$$;

CREATE OR REPLACE TRIGGER track_search_update
    AFTER UPDATE OF id, sc_track_id, uploader_sc_user_id, title, title_normalized, metadata_artist, uploader_username, duration_ms, play_count_sc, sharing, deleted_at, superseded_by
    ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived() AND (OLD.id IS DISTINCT FROM NEW.id
           OR OLD.sc_track_id IS DISTINCT FROM NEW.sc_track_id
           OR OLD.uploader_sc_user_id IS DISTINCT FROM NEW.uploader_sc_user_id
           OR OLD.title IS DISTINCT FROM NEW.title
           OR OLD.title_normalized IS DISTINCT FROM NEW.title_normalized
           OR OLD.metadata_artist IS DISTINCT FROM NEW.metadata_artist
           OR OLD.uploader_username IS DISTINCT FROM NEW.uploader_username
           OR OLD.duration_ms IS DISTINCT FROM NEW.duration_ms
           OR OLD.play_count_sc IS DISTINCT FROM NEW.play_count_sc
           OR OLD.sharing IS DISTINCT FROM NEW.sharing
           OR OLD.deleted_at IS DISTINCT FROM NEW.deleted_at
           OR OLD.superseded_by IS DISTINCT FROM NEW.superseded_by))
    EXECUTE FUNCTION public.track_search_sync();

CREATE OR REPLACE TRIGGER track_search_row
    AFTER INSERT OR DELETE ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.track_search_sync();

ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER track_search_update;

ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER track_search_row;
