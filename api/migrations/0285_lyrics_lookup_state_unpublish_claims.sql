SET LOCAL lock_timeout = '5s';

SELECT public.scd_unpublish_column('lyrics_lookup_state', 'attempts');
SELECT public.scd_unpublish_column('lyrics_lookup_state', 'last_attempt_at');
SELECT public.scd_unpublish_column('lyrics_lookup_state', 'claim_job_id');
SELECT public.scd_unpublish_column('lyrics_lookup_state', 'claim_job_generation');
SELECT public.scd_unpublish_column('lyrics_lookup_state', 'claim_job_lease_id');
SELECT public.scd_unpublish_column('lyrics_lookup_state', 'claim_state_generation');
SELECT public.scd_unpublish_column('lyrics_lookup_state', 'claim_expires_at');
