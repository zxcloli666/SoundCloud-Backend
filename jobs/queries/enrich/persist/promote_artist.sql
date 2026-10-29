UPDATE artists
SET mb_artist_id     = COALESCE(mb_artist_id, $2),
    genius_artist_id = COALESCE(genius_artist_id, $3),
    source           = CASE WHEN $4 THEN $5 ELSE source END,
    confidence       = CASE WHEN $4 THEN $6 ELSE confidence END,
    updated_at       = now()
WHERE id = $1
  AND (mb_artist_id, genius_artist_id, source, confidence) IS DISTINCT FROM (
      COALESCE(mb_artist_id, $2),
      COALESCE(genius_artist_id, $3),
      CASE WHEN $4 THEN $5 ELSE source END,
      CASE WHEN $4 THEN $6 ELSE confidence END
  )
