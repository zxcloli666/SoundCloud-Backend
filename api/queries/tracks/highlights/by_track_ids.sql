SELECT track_id, start_ms, length_ms
FROM track_highlights
WHERE track_id = ANY ($1)
