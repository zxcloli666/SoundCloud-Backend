SET LOCAL lock_timeout = '5s';

CREATE TABLE IF NOT EXISTS public.track_storage_failures (
    track_id uuid PRIMARY KEY REFERENCES public.tracks(id) ON DELETE CASCADE,
    failed_at timestamptz NOT NULL
);

CREATE INDEX IF NOT EXISTS track_storage_failures_failed_at_idx
    ON public.track_storage_failures (failed_at, track_id);

CREATE OR REPLACE FUNCTION public.track_storage_failures_sync()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog, pg_temp
AS $f$
BEGIN
    IF NEW.storage_state = 'failed' THEN
        INSERT INTO public.track_storage_failures AS failure (track_id, failed_at)
        VALUES (NEW.id, now())
        ON CONFLICT (track_id) DO UPDATE
        SET failed_at = EXCLUDED.failed_at;
    ELSIF TG_OP = 'UPDATE' AND OLD.storage_state = 'failed' THEN
        DELETE FROM public.track_storage_failures WHERE track_id = NEW.id;
    END IF;
    RETURN NULL;
END
$f$;

CREATE OR REPLACE TRIGGER track_storage_failures_sync
AFTER INSERT OR UPDATE OF storage_state ON public.tracks
FOR EACH ROW
WHEN (public.scd_derived())
EXECUTE FUNCTION public.track_storage_failures_sync();

ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER track_storage_failures_sync;

DO $fill$
BEGIN
    IF public.scd_role() = 'core' THEN
        INSERT INTO public.track_storage_failures (track_id, failed_at)
        SELECT track.id, track.updated_at
        FROM public.tracks AS track
        WHERE track.storage_state = 'failed'
        ON CONFLICT (track_id) DO NOTHING;
    END IF;
END
$fill$;
