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
    next_at = now(),
    last_error = 'unanswered',
    payload = NULL,
    updated_at = now()
WHERE stage = 'dispatched'
  AND (
      updated_at < now() - make_interval(secs => $1)
      OR (payload IS NULL AND updated_at < now() - make_interval(secs => $2))
  )
