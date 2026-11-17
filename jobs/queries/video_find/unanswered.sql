UPDATE track_video_dispatch
SET stage = 'retry',
    next_at = now(),
    last_error = 'unanswered',
    payload = NULL,
    updated_at = now()
WHERE stage = 'dispatched'
  AND updated_at < now() - make_interval(secs => $1)
