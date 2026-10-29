UPDATE tracks
SET album_id = COALESCE(album_id, $2)
WHERE id = $1
  AND album_id IS DISTINCT FROM COALESCE(album_id, $2)
