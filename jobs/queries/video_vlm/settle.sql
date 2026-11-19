WITH research AS (
    SELECT $3::text = 'found'
               OR (
                   $3::text IN ('none', 'static')
                   AND EXISTS (
                       SELECT 1
                       FROM track_videos AS video
                       WHERE video.sc_track_id = $1
                         AND video.kind IN ('clip', 'loop')
                   )
               ) AS found,
           (
               SELECT COALESCE(track.release_date::timestamptz, track.sc_created_at)
               FROM tracks AS track
               WHERE track.sc_track_id = $1
           ) > now() - make_interval(secs => $8) AS fresh
)
UPDATE track_video_dispatch
SET stage = CASE WHEN research.found THEN 'found' ELSE $3 END,
    next_at = now() + make_interval(
        secs => CASE
            WHEN NOT research.found THEN $4::float8
            WHEN research.fresh THEN $6::float8
            ELSE $7::float8
        END
    ),
    last_error = $5,
    payload = NULL,
    updated_at = now()
FROM research
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'vlm_dispatched'
RETURNING sc_track_id
