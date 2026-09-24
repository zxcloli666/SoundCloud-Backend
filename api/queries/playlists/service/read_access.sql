SELECT sharing = 'public' OR owner_sc_user_id = $2 AS "can_read?",
       deleted_at IS NOT NULL AS "deleted!",
       sc_write_confirmed OR COALESCE(sc_desired->>'sharing', '') <> 'private' AS "secret_ready!"
FROM playlists WHERE urn = $1
