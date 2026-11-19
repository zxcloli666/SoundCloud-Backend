SELECT source, source_ref, s3_key
FROM track_videos
WHERE sc_track_id = $1
  AND kind IN ('clip', 'loop')
ORDER BY kind
