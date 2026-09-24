SELECT sharing = 'public' AND deleted_at IS NULL AS "public!"
FROM tracks
WHERE sc_track_id = $1
