UPDATE track_video_dispatch
SET stage = 'pending_vlm',
    next_at = now(),
    last_error = $3,
    payload = jsonb_set(payload - 'vlm_gpu', '{gpu_failed}', 'true'),
    updated_at = now()
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'vlm_dispatched'
  AND payload IS NOT NULL
RETURNING sc_track_id
