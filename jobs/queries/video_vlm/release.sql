UPDATE track_video_dispatch
SET stage = 'pending_vlm',
    next_at = now() + make_interval(secs => $3),
    last_error = $4,
    updated_at = now()
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'vlm_dispatched'
