SET LOCAL lock_timeout = '5s';

SELECT public.scd_unpublish_column('tracks', 'work_key');
SELECT public.scd_unpublish_column('tracks', 'recording_key');
SELECT public.scd_unpublish_column('tracks', 'work_normalizer_version');
