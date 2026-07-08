UPDATE tracks
SET genius_song_id = $2
WHERE id = $1
  AND genius_song_id IS NULL
