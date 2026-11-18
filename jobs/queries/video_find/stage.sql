UPDATE track_video_dispatch
SET payload = $3,
    updated_at = now()
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'dispatched'
