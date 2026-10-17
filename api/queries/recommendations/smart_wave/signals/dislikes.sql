SELECT sc_track_id AS id, (EXTRACT(EPOCH FROM created_at) * 1000000)::int8 AS "at!"
FROM disliked_tracks
WHERE sc_user_id = ANY ($1)
ORDER BY created_at DESC LIMIT $2
