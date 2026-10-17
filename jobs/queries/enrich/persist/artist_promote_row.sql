SELECT source, confidence, mb_artist_id, genius_artist_id
FROM artists
WHERE id = $1
