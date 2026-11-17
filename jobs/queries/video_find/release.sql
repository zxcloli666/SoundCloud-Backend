UPDATE track_video_dispatch
SET stage = 'retry',
    attempts = GREATEST(attempts - 1, 0),
    next_at = now() + make_interval(secs => $3),
    last_error = $4,
    payload = NULL,
    updated_at = now()
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'dispatched'
