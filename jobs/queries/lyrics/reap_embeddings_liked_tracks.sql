SELECT track.sc_track_id AS "sc_track_id!"
FROM tracks AS track
WHERE track.sc_track_id = ANY ($1::text[])
  AND track.index_priority = $2
