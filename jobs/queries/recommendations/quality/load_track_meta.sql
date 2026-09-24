SELECT track.sc_track_id,
       track.title,
       track.genre,
       track.duration_ms,
       counters.play_count AS "play_count?",
       counters.likes_count AS "likes_count?"
FROM tracks AS track
LEFT JOIN sc_track_counters AS counters
  ON counters.sc_track_id = track.sc_track_id
WHERE track.sc_track_id = ANY ($1)
