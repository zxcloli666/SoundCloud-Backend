UPDATE tracks
SET primary_artist_id = NULL
WHERE id = ANY ($1)
