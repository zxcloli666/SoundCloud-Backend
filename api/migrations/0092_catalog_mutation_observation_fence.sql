ALTER TABLE tracks
    ADD COLUMN IF NOT EXISTS sc_mutation_observation bigint NOT NULL DEFAULT 0;

ALTER TABLE playlists
    ADD COLUMN IF NOT EXISTS sc_mutation_observation bigint NOT NULL DEFAULT 0;

CREATE OR REPLACE FUNCTION catalog_observation_is_current(
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
    SELECT incoming_observation > mutation_observation
       AND (
           incoming_observation > stored_observation
           OR (desired = '{}'::jsonb AND incoming_modified > stored_modified)
       )
       AND (stored_modified IS NULL OR incoming_modified >= stored_modified)
       AND (desired = '{}'::jsonb OR (write_confirmed AND incoming @> desired))
$$;

DROP FUNCTION catalog_observation_is_current(bigint, bigint, timestamptz, timestamptz, jsonb, boolean, jsonb);

WITH pending AS MATERIALIZED (
    SELECT queued.target_urn,
           queued.payload->>'sharing' AS sharing,
           nextval('catalog_metadata_clock') AS observation
    FROM sync_queue AS queued
    WHERE queued.action_type = 'track_sharing'
      AND queued.payload->>'sharing' IN ('public', 'private')
)
UPDATE tracks AS track
SET sharing = pending.sharing,
    sc_desired = track.sc_desired || jsonb_build_object('sharing', pending.sharing),
    sc_observation = pending.observation,
    sc_mutation_observation = pending.observation,
    sc_write_confirmed = false
FROM pending
WHERE track.urn = pending.target_urn;

WITH pending AS MATERIALIZED (
    SELECT queued.target_urn,
           queued.payload->>'sharing' AS sharing,
           nextval('catalog_metadata_clock') AS observation
    FROM sync_queue AS queued
    WHERE queued.action_type = 'playlist_sharing'
      AND queued.payload->>'sharing' IN ('public', 'private')
)
UPDATE playlists AS playlist
SET sharing = pending.sharing,
    sc_desired = playlist.sc_desired || jsonb_build_object('sharing', pending.sharing),
    sc_observation = pending.observation,
    sc_mutation_observation = pending.observation,
    sc_write_confirmed = false
FROM pending
WHERE playlist.urn = pending.target_urn;
