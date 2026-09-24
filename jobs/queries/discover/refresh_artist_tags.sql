WITH tag_counts AS (
    SELECT primary_artist_id AS artist_id,
           lower(trim(genre)) AS tag,
           count(*) AS track_count
    FROM tracks
    WHERE primary_artist_id IS NOT NULL
      AND genre IS NOT NULL
      AND trim(genre) <> ''
    GROUP BY primary_artist_id, lower(trim(genre))
), ranked AS (
    SELECT artist_id,
           tag,
           row_number() OVER (
               PARTITION BY artist_id
               ORDER BY track_count DESC, tag
           ) AS position
    FROM tag_counts
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
    GROUP BY artist.id
)
UPDATE artists AS artist
SET tags = aggregate.tags
FROM aggregate
WHERE artist.id = aggregate.id
  AND artist.tags IS DISTINCT FROM aggregate.tags
