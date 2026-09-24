SELECT DISTINCT dislike.sc_track_id
FROM disliked_tracks AS dislike
JOIN tracks AS track
  ON track.sc_track_id = dislike.sc_track_id
WHERE track.indexed_at IS NOT NULL
  AND track.index_state = 'indexed'
  AND track.storage_state <> 'too_long'
  AND track.needs_duration_resolve = false
ORDER BY dislike.sc_track_id
LIMIT $1
