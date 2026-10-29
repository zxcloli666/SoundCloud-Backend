UPDATE artists
SET mb_artist_id     = COALESCE(mb_artist_id, $2),
    genius_artist_id = COALESCE(genius_artist_id, $3),
    updated_at       = now()
WHERE id = $1
  AND (mb_artist_id, genius_artist_id) IS DISTINCT FROM
      (COALESCE(mb_artist_id, $2), COALESCE(genius_artist_id, $3))
