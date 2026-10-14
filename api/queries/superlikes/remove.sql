DELETE
FROM user_superlikes
WHERE user_id = ANY ($1)
  AND sc_track_id = $2
