UPDATE track_video_gen
SET stage = $4,
    step = $5,
    shot = $6,
    redo = $7,
    faults = $8,
    plan = $9,
    state = $10,
    rejections = $11,
    reason = $12,
    sent_at = CASE WHEN $13::bool THEN now() END,
    next_at = now() + $14::float8 * interval '1 second',
    updated_at = now()
WHERE sc_track_id = $1
  AND attempt = $2
  AND step = $3
