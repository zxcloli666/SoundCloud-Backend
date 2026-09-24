UPDATE artists
SET merged_into = $2, mb_artist_id = NULL, genius_artist_id = NULL
WHERE id = $1
