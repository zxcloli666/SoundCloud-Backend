UPDATE track_video_dispatch
SET payload = jsonb_set(payload, '{vlm_gpu}', 'true')
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'vlm_dispatched'
  AND payload IS NOT NULL
