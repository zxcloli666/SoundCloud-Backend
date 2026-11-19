SELECT payload
FROM track_video_dispatch
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'vlm_dispatched'
