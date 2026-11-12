WITH ranked AS (
    SELECT artist_id,
           tag,
           row_number() OVER (
               PARTITION BY artist_id
               ORDER BY track_count DESC, tag
           ) AS position
    FROM artist_genre_counts
    WHERE artist_id BETWEEN $1 AND $2
), aggregate AS (
    SELECT artist.id,
           COALESCE(
               array_agg(ranked.tag ORDER BY ranked.position)
                   FILTER (WHERE ranked.position <= 3),
               '{}'::text[]
           ) AS tags
    FROM artists AS artist
    LEFT JOIN ranked ON ranked.artist_id = artist.id
    WHERE artist.merged_into IS NULL
      AND artist.id BETWEEN $1 AND $2
    GROUP BY artist.id
)
UPDATE artists AS artist
SET tags = aggregate.tags
FROM aggregate
WHERE artist.id = aggregate.id
  AND artist.tags IS DISTINCT FROM aggregate.tags
