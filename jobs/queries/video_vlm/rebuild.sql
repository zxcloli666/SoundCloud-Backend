UPDATE track_video_dispatch
SET stage = CASE WHEN attempts >= $4 THEN 'none' ELSE 'retry' END,
    next_at = now() + make_interval(secs => CASE WHEN attempts >= $4 THEN $6::float8 ELSE $3::float8 * attempts END),
    last_error = $5,
    payload = NULL,
    updated_at = now()
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'vlm_dispatched'
RETURNING stage AS "stage!"
