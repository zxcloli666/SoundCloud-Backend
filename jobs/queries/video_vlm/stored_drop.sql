DELETE FROM track_videos
WHERE sc_track_id = $1
  AND s3_key = $2
  AND kind IN ('clip', 'loop')
RETURNING kind
