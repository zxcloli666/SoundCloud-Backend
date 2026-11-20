UPDATE track_video_dispatch
SET stage = 'pending_vlm',
    next_at = now(),
    last_error = CASE WHEN payload ? 'vlm_gpu' THEN 'gpu_unanswered' ELSE 'unanswered' END,
    payload = CASE
        WHEN payload ? 'vlm_gpu' THEN jsonb_set(payload - 'vlm_gpu', '{gpu_failed}', 'true')
        ELSE payload
    END,
    updated_at = now()
WHERE stage = 'vlm_dispatched'
  AND updated_at < now() - make_interval(
      secs => CASE WHEN payload ? 'vlm_gpu' THEN $2::float8 ELSE $1::float8 END
  )
