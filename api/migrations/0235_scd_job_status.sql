SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_job_status_kinds()
RETURNS text[]
LANGUAGE sql
IMMUTABLE
AS $f$
SELECT ARRAY['catalog.refresh', 'catalog.collection', 'playlists.observe_shadow']
$f$;

CREATE OR REPLACE FUNCTION public.scd_job_status_state(leased boolean, failed boolean)
RETURNS text
LANGUAGE sql
IMMUTABLE
AS $f$
SELECT CASE WHEN leased THEN 'running' WHEN failed THEN 'retry_wait' ELSE 'queued' END
$f$;

CREATE OR REPLACE FUNCTION public.scd_job_status_sync()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    IF TG_OP = 'DELETE' THEN
        IF OLD.dedup_key IS NOT NULL THEN
            DELETE FROM public.job_status s WHERE s.kind = OLD.kind AND s.dedup_key = OLD.dedup_key;
        END IF;
        RETURN NULL;
    END IF;
    IF NEW.dedup_key IS NULL THEN
        RETURN NULL;
    END IF;
    INSERT INTO public.job_status AS s (kind, dedup_key, state, available_at, updated_at)
    VALUES (NEW.kind, NEW.dedup_key, public.scd_job_status_state(NEW.lease_id IS NOT NULL, NEW.last_error IS NOT NULL), NEW.available_at, now())
    ON CONFLICT (kind, dedup_key) DO UPDATE SET state = EXCLUDED.state, available_at = EXCLUDED.available_at, updated_at = EXCLUDED.updated_at
    WHERE s.updated_at <= EXCLUDED.updated_at AND (s.state, s.available_at) IS DISTINCT FROM (EXCLUDED.state, EXCLUDED.available_at);
    RETURN NULL;
END
$f$;

CREATE OR REPLACE TRIGGER zz_job_status_insert AFTER INSERT ON public.background_jobs
FOR EACH ROW WHEN (NEW.kind = ANY (public.scd_job_status_kinds()) AND public.scd_mode() = 'split' AND public.scd_role() = 'core')
EXECUTE FUNCTION public.scd_job_status_sync();

CREATE OR REPLACE TRIGGER zz_job_status_update AFTER UPDATE OF lease_id, last_error, available_at ON public.background_jobs
FOR EACH ROW WHEN (NEW.kind = ANY (public.scd_job_status_kinds()) AND public.scd_mode() = 'split' AND public.scd_role() = 'core'
    AND (OLD.lease_id IS DISTINCT FROM NEW.lease_id OR OLD.last_error IS DISTINCT FROM NEW.last_error OR OLD.available_at IS DISTINCT FROM NEW.available_at))
EXECUTE FUNCTION public.scd_job_status_sync();

CREATE OR REPLACE TRIGGER zz_job_status_delete AFTER DELETE ON public.background_jobs
FOR EACH ROW WHEN (OLD.kind = ANY (public.scd_job_status_kinds()) AND public.scd_mode() = 'split' AND public.scd_role() = 'core')
EXECUTE FUNCTION public.scd_job_status_sync();

CREATE OR REPLACE FUNCTION public.scd_job_status_backfill(after uuid DEFAULT NULL, batch integer DEFAULT 1000, OUT last_id uuid, OUT added bigint)
LANGUAGE plpgsql
SET search_path = public, pg_catalog
SET lock_timeout = '2s'
AS $f$
BEGIN
    added := 0;
    IF public.scd_mode() <> 'split' OR public.scd_role() <> 'core' THEN
        RETURN;
    END IF;
    WITH page AS (
        SELECT j.id, j.kind, j.dedup_key, j.lease_id, j.last_error, j.available_at
        FROM public.background_jobs j
        WHERE (after IS NULL OR j.id > after) AND j.kind = ANY (public.scd_job_status_kinds())
        ORDER BY j.id
        LIMIT batch
    ), ins AS (
        INSERT INTO public.job_status (kind, dedup_key, state, available_at, updated_at)
        SELECT p.kind, p.dedup_key, public.scd_job_status_state(p.lease_id IS NOT NULL, p.last_error IS NOT NULL), p.available_at, now()
        FROM page p
        WHERE p.dedup_key IS NOT NULL
        ON CONFLICT (kind, dedup_key) DO NOTHING
        RETURNING 1
    )
    SELECT (SELECT p.id FROM page p ORDER BY p.id DESC LIMIT 1), (SELECT count(*) FROM ins) INTO last_id, added;
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_job_status_prune(batch integer DEFAULT 1000)
RETURNS bigint
LANGUAGE plpgsql
SET search_path = public, pg_catalog
SET lock_timeout = '2s'
AS $f$
DECLARE
    removed bigint;
BEGIN
    IF public.scd_mode() <> 'split' OR public.scd_role() <> 'core' THEN
        RETURN 0;
    END IF;
    WITH d AS (
        DELETE FROM public.job_status t WHERE t.ctid IN (
            SELECT s.ctid FROM public.job_status s
            WHERE s.kind = ANY (public.scd_job_status_kinds())
              AND NOT EXISTS (SELECT 1 FROM public.background_jobs j WHERE j.kind = s.kind AND j.dedup_key = s.dedup_key)
            LIMIT batch FOR UPDATE SKIP LOCKED
        ) RETURNING 1
    ) SELECT count(*) INTO removed FROM d;
    RETURN removed;
END
$f$;
