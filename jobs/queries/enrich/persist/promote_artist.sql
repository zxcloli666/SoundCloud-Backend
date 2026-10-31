UPDATE artists AS artist
SET mb_artist_id     = next.mb_artist_id,
    genius_artist_id = next.genius_artist_id,
    source           = next.source,
    confidence       = next.confidence,
    updated_at       = now()
FROM (
    SELECT COALESCE(
               current.mb_artist_id,
               CASE WHEN NOT EXISTS (
                   SELECT 1 FROM artists other
                   WHERE other.mb_artist_id = $2::text AND other.id <> current.id
               ) THEN $2::text END
           ) AS mb_artist_id,
           COALESCE(
               current.genius_artist_id,
               CASE WHEN NOT EXISTS (
                   SELECT 1 FROM artists other
                   WHERE other.genius_artist_id = $3::text AND other.id <> current.id
               ) THEN $3::text END
           ) AS genius_artist_id,
           CASE WHEN $4 THEN $5 ELSE current.source END AS source,
           CASE WHEN $4 THEN $6 ELSE current.confidence END AS confidence
    FROM artists current
    WHERE current.id = $1
) next
WHERE artist.id = $1
  AND (artist.mb_artist_id, artist.genius_artist_id, artist.source, artist.confidence)
      IS DISTINCT FROM (next.mb_artist_id, next.genius_artist_id, next.source, next.confidence)
