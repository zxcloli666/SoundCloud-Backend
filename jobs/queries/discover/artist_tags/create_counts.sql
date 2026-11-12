CREATE TEMP TABLE artist_genre_counts AS
SELECT primary_artist_id AS artist_id,
       lower(trim(genre)) AS tag,
       count(*) AS track_count
FROM tracks
WHERE primary_artist_id IS NOT NULL
  AND genre IS NOT NULL
  AND trim(genre) <> ''
GROUP BY primary_artist_id, lower(trim(genre))
