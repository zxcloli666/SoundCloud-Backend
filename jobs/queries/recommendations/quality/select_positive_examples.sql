SELECT track.sc_track_id AS "sc_track_id!"
FROM user_events AS event
JOIN tracks AS track
  ON track.sc_track_id = event.sc_track_id
WHERE event.created_at >= $1
  AND event.event_type = ANY ($2)
  AND track.indexed_at IS NOT NULL
  AND track.index_state = 'indexed'
  AND track.storage_state <> 'too_long'
  AND track.needs_duration_resolve = false
  AND NOT EXISTS (
      SELECT 1
      FROM disliked_tracks AS dislike
      WHERE dislike.sc_track_id = track.sc_track_id
  )
GROUP BY track.sc_track_id
HAVING count(DISTINCT event.sc_user_id) >= $3
ORDER BY md5(track.sc_track_id), track.sc_track_id
LIMIT $4
