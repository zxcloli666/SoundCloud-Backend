UPDATE track_videos
SET verify = verify || jsonb_build_object('vlm', $3::jsonb)
WHERE sc_track_id = $1
  AND s3_key = $2
  AND kind IN ('clip', 'loop')
