UPDATE track_video_dispatch
SET stage = 'pending_vlm',
    next_at = now(),
    last_error = 'unanswered',
    updated_at = now()
WHERE stage = 'vlm_dispatched'
  AND updated_at < now() - make_interval(secs => $1)
