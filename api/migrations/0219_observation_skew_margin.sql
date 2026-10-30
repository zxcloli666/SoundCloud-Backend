SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_observation_skew()
RETURNS bigint
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$
    SELECT 1000000::bigint << 4
$$;

CREATE OR REPLACE FUNCTION public.catalog_observation_is_current(
    stored_observation bigint,
    mutation_observation bigint,
    incoming_observation bigint,
    stored_modified timestamptz,
    incoming_modified timestamptz,
    desired jsonb,
    write_confirmed boolean,
    incoming jsonb
) RETURNS boolean
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$
    SELECT incoming_observation > mutation_observation + public.scd_observation_skew()
       AND (
           incoming_observation > stored_observation
           OR (desired = '{}'::jsonb AND incoming_modified > stored_modified)
       )
       AND (stored_modified IS NULL OR incoming_modified >= stored_modified)
       AND (desired = '{}'::jsonb OR (write_confirmed AND incoming @> desired))
$$;
