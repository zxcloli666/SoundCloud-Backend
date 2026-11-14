SELECT t.sc_track_id
FROM tracks t
         LEFT JOIN track_sync_stamps stamp ON stamp.track_id = t.id
WHERE t.cover_of_artist_id = $1
ORDER BY COALESCE(t.play_count_sc, 0) DESC, GREATEST(t.sc_synced_at, stamp.sc_synced_at) DESC
LIMIT $2 OFFSET $3
