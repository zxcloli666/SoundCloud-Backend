SELECT sharing = 'public' OR uploader_sc_user_id = $2 AS "can_read?",
       deleted_at IS NOT NULL AS "deleted!",
       sc_mutation_observation,
       sc_write_confirmed OR COALESCE(sc_desired->>'sharing', '') <> 'private' AS "secret_ready!"
FROM tracks
WHERE sc_track_id = $1
