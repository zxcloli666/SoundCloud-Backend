SELECT sc_track_id, kind, s3_key, offset_ms, loop AS looped
FROM track_videos
WHERE sc_track_id = ANY ($1)
ORDER BY sc_track_id, CASE kind WHEN 'clip' THEN 0 WHEN 'loop' THEN 1 ELSE 2 END
