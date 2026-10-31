UPDATE tracks
SET primary_artist_id = NULL,
    enrich_state = NULL
WHERE id = $1
  AND primary_artist_id = $2
