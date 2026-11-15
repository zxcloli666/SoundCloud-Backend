SET LOCAL lock_timeout = '5s';

SELECT public.scd_unpublish_column('tracks', 'enrich_next_run_at');
SELECT public.scd_unpublish_column('tracks', 'enrich_locked_at');
SELECT public.scd_unpublish_column('tracks', 'enrich_attempts');
SELECT public.scd_unpublish_column('tracks', 'enrich_error');
