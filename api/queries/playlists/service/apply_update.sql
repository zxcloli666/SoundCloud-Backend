WITH observation AS MATERIALIZED (
    SELECT nextval('catalog_metadata_clock') AS sequence
)
UPDATE playlists AS playlist
SET title = CASE WHEN $3::jsonb ? 'title' THEN $3->>'title' ELSE title END,
    title_normalized = CASE WHEN $3::jsonb ? 'title_normalized' THEN $3->>'title_normalized' ELSE title_normalized END,
    description = CASE WHEN $3::jsonb ? 'description' THEN $3->>'description' ELSE description END,
    genre = CASE WHEN $3::jsonb ? 'genre' THEN $3->>'genre' ELSE genre END,
    tags = CASE WHEN $3::jsonb ? 'tags' THEN ARRAY(SELECT jsonb_array_elements_text($3->'tags')) ELSE tags END,
    sharing = CASE WHEN $3::jsonb ? 'sharing' THEN $3->>'sharing' ELSE sharing END,
    label_name = CASE WHEN $3::jsonb ? 'label_name' THEN $3->>'label_name' ELSE label_name END,
    permalink_url = CASE WHEN $3::jsonb ? 'permalink_url' THEN $3->>'permalink_url' ELSE permalink_url END,
    playlist_type = CASE WHEN $3::jsonb ? 'playlist_type' THEN $3->>'playlist_type' ELSE playlist_type END,
    release_date = CASE WHEN $3::jsonb ? 'release_date' THEN ($3->>'release_date')::date ELSE release_date END,
    release_year = CASE WHEN $3::jsonb ? 'release_year' THEN ($3->>'release_year')::smallint ELSE release_year END,
    sc_metadata = playlist.sc_metadata || COALESCE($3->'sc_metadata', '{}'),
    sc_desired = playlist.sc_desired || $3::jsonb ||
        CASE WHEN $3::jsonb ? 'sc_metadata' THEN jsonb_build_object('sc_metadata',
            COALESCE(playlist.sc_desired->'sc_metadata', '{}') || ($3->'sc_metadata')) ELSE '{}'::jsonb END,
    sc_observation = observation.sequence,
    sc_mutation_observation = observation.sequence,
    sc_write_confirmed = false,
    updated_at = clock_timestamp()
FROM observation
WHERE playlist.urn = $1 AND playlist.owner_sc_user_id = $2 AND playlist.deleted_at IS NULL
RETURNING playlist.urn
