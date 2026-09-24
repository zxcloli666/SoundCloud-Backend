CREATE SEQUENCE IF NOT EXISTS catalog_metadata_clock AS bigint;

ALTER TABLE tracks
    ADD COLUMN IF NOT EXISTS sc_observation bigint NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS sc_desired jsonb NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS sc_write_confirmed boolean NOT NULL DEFAULT false;

ALTER TABLE playlists
    ADD COLUMN IF NOT EXISTS sc_observation bigint NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS sc_desired jsonb NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS sc_write_confirmed boolean NOT NULL DEFAULT false;

CREATE OR REPLACE FUNCTION catalog_observation_is_current(
    stored_observation bigint,
    incoming_observation bigint,
    stored_modified timestamptz,
    incoming_modified timestamptz,
    desired jsonb,
    write_confirmed boolean,
    incoming jsonb
) RETURNS boolean
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$
    SELECT incoming_observation > stored_observation
       AND (stored_modified IS NULL OR incoming_modified >= stored_modified)
       AND (desired = '{}'::jsonb OR (write_confirmed AND incoming @> desired))
$$;
