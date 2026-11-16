SET LOCAL lock_timeout = '5s';

SELECT public.scd_unpublish_column('artists', 'interest_score');
