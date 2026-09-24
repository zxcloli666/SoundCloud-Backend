WITH observation AS MATERIALIZED (
    SELECT nextval('catalog_metadata_clock') AS sequence
)
UPDATE tracks AS track
SET sharing = 'private',
    sc_desired = '{}',
    deleted_at = COALESCE(track.deleted_at, clock_timestamp()),
    sc_observation = observation.sequence,
    sc_mutation_observation = observation.sequence,
    sc_write_confirmed = false,
    updated_at = clock_timestamp()
FROM observation
WHERE track.sc_track_id = $1
  AND track.uploader_sc_user_id = $2
RETURNING track.urn
