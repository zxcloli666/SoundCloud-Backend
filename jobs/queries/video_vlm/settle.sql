UPDATE track_video_dispatch
SET stage = $3,
    next_at = now() + make_interval(secs => $4),
    last_error = $5,
    payload = NULL,
    updated_at = now()
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'vlm_dispatched'
RETURNING sc_track_id
