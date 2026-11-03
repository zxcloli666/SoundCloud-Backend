-- no-transaction
DO $do$
DECLARE
    page record;
    after uuid;
BEGIN
    IF public.scd_mode() = 'split' AND public.scd_role() <> 'core' THEN
        RETURN;
    END IF;
    LOOP
        SELECT b.last_id, b.added INTO page FROM public.scd_job_status_backfill(after, 1000) b;
        COMMIT;
        EXIT WHEN page.last_id IS NULL;
        after := page.last_id;
    END LOOP;
END
$do$;
