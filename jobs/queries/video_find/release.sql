UPDATE track_video_dispatch AS dispatch
SET stage = CASE
        WHEN EXISTS (
            SELECT 1
            FROM track_videos AS video
            WHERE video.sc_track_id = dispatch.sc_track_id
              AND video.kind IN ('clip', 'loop')
        ) THEN 'found'
        ELSE 'retry'
    END,
    attempts = GREATEST(attempts - 1, 0),
    next_at = now() + make_interval(secs => $3),
    last_error = $4,
    payload = NULL,
    updated_at = now()
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'dispatched'
