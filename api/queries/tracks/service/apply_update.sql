WITH observation AS MATERIALIZED (
    SELECT nextval('catalog_metadata_clock') AS sequence
)
UPDATE tracks AS track
SET title = CASE WHEN $3::jsonb ? 'title' THEN $3->>'title' ELSE track.title END,
    title_normalized = CASE WHEN $3 ? 'title_normalized' THEN $3->>'title_normalized' ELSE track.title_normalized END,
    description = CASE WHEN $3 ? 'description' THEN $3->>'description' ELSE track.description END,
    genre = CASE WHEN $3 ? 'genre' THEN $3->>'genre' ELSE track.genre END,
    tags = CASE WHEN $3 ? 'tags' THEN ARRAY(SELECT jsonb_array_elements_text($3->'tags')) ELSE track.tags END,
    sharing = CASE WHEN $3 ? 'sharing' THEN $3->>'sharing' ELSE track.sharing END,
    isrc = CASE WHEN $3 ? 'isrc' THEN $3->>'isrc' ELSE track.isrc END,
    metadata_artist = CASE WHEN $3 ? 'metadata_artist' THEN $3->>'metadata_artist' ELSE track.metadata_artist END,
    release_date = CASE WHEN $3 ? 'release_date' THEN ($3->>'release_date')::date ELSE track.release_date END,
    release_year = CASE WHEN $3 ? 'release_year' THEN ($3->>'release_year')::smallint ELSE track.release_year END,
    sc_metadata = track.sc_metadata || COALESCE($3->'sc_metadata', '{}'),
    sc_desired = track.sc_desired || ($3 - 'sc_metadata') || CASE
        WHEN $3 ? 'sc_metadata' THEN jsonb_build_object('sc_metadata', COALESCE(track.sc_desired->'sc_metadata', '{}') || ($3->'sc_metadata'))
        ELSE '{}'::jsonb
    END,
    sc_observation = observation.sequence,
    sc_mutation_observation = observation.sequence,
    sc_write_confirmed = false,
    updated_at = clock_timestamp()
FROM observation
WHERE track.sc_track_id = $1
  AND track.uploader_sc_user_id = $2
  AND track.deleted_at IS NULL
RETURNING track.urn
