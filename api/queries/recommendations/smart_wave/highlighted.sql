SELECT track.sc_track_id
FROM tracks AS track
JOIN track_highlights AS highlight ON highlight.track_id = track.id
WHERE track.sc_track_id = ANY ($1)
