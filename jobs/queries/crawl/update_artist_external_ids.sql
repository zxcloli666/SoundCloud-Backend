UPDATE artists AS artist
SET mb_artist_id     = next.mb_artist_id,
    genius_artist_id = next.genius_artist_id,
    updated_at       = now()
FROM (
    SELECT COALESCE(
               cur.mb_artist_id,
               CASE WHEN NOT EXISTS (
                   SELECT 1 FROM artists other
                   WHERE other.mb_artist_id = $2::text AND other.id <> cur.id
               ) THEN $2::text END
           ) AS mb_artist_id,
           COALESCE(
               cur.genius_artist_id,
               CASE WHEN NOT EXISTS (
                   SELECT 1 FROM artists other
                   WHERE other.genius_artist_id = $3::text AND other.id <> cur.id
               ) THEN $3::text END
           ) AS genius_artist_id
    FROM artists cur
    WHERE cur.id = $1
) next
WHERE artist.id = $1
  AND (artist.mb_artist_id, artist.genius_artist_id)
      IS DISTINCT FROM (next.mb_artist_id, next.genius_artist_id)
