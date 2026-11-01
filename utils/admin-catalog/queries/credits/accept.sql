UPDATE track_artists
SET source = 'sc_verified',
    confidence = greatest(confidence, 0.9),
    evidence = 'manual'
WHERE track_id = $1
  AND artist_id = $2
  AND role = $3
  AND evidence IN (
      'uploader_name',
      'title_heuristic',
      'ai_inference',
      'unattributed'
  )
