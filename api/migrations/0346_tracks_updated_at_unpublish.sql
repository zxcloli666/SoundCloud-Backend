SET LOCAL lock_timeout = '5s';

SELECT public.scd_unpublish_column('tracks', 'updated_at');
