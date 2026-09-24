WITH observation AS MATERIALIZED (
    SELECT nextval('catalog_metadata_clock') AS sequence
)
UPDATE playlists AS playlist
SET sharing = 'private', sc_desired = '{}',
    deleted_at = COALESCE(playlist.deleted_at, clock_timestamp()),
    sc_observation = observation.sequence,
    sc_mutation_observation = observation.sequence,
    sc_write_confirmed = false,
    updated_at = clock_timestamp()
FROM observation
WHERE playlist.urn = $1 AND playlist.owner_sc_user_id = $2
RETURNING playlist.urn
