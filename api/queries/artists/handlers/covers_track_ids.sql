SELECT t.sc_track_id
FROM tracks t
WHERE t.cover_of_artist_id = $1
ORDER BY COALESCE(t.play_count_sc, 0) DESC, t.sc_synced_at DESC
LIMIT $2 OFFSET $3
