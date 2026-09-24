WITH observation AS MATERIALIZED (
    SELECT nextval('catalog_metadata_clock') AS sequence
)
UPDATE tracks
SET sc_observation = observation.sequence,
    sc_mutation_observation = observation.sequence,
    sc_write_confirmed = true,
    updated_at = clock_timestamp()
FROM observation
WHERE sc_track_id = $1
  AND deleted_at IS NULL
  AND sc_desired @> $2::jsonb
