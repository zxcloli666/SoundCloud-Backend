SELECT sc_track_id, stage, attempt, step, shot, redo, faults, plan, state, rejections
FROM track_video_gen
WHERE sc_track_id = $1
  AND attempt = $2
  AND step = $3
  AND sent_at IS NOT NULL
