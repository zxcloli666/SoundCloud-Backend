WITH primary_counts AS (
    SELECT artist_id, count(*)::integer AS track_count
    FROM track_artists
    WHERE role = 'primary'
    GROUP BY artist_id
), featured_counts AS (
    SELECT artist_id, count(DISTINCT track_id)::integer AS track_count
    FROM track_artists
    WHERE role IN ('featured', 'remixer')
    GROUP BY artist_id
), album_counts AS (
    SELECT artist_id, count(DISTINCT album_id)::integer AS album_count
    FROM (
        SELECT primary_artist_id AS artist_id, id AS album_id
        FROM albums
        WHERE primary_artist_id IS NOT NULL
        UNION
        SELECT artist_id, album_id
        FROM album_artists
    ) AS linked
    GROUP BY artist_id
), aggregate AS (
    SELECT COALESCE(primary_counts.artist_id, featured_counts.artist_id, album_counts.artist_id) AS artist_id,
           COALESCE(primary_counts.track_count, 0) AS primary_count,
           COALESCE(featured_counts.track_count, 0) AS featured_count,
           COALESCE(album_counts.album_count, 0) AS album_count
    FROM primary_counts
    FULL JOIN featured_counts USING (artist_id)
    FULL JOIN album_counts USING (artist_id)
)
UPDATE artists AS artist
SET track_count_primary = COALESCE(aggregate.primary_count, 0),
    track_count_featured = COALESCE(aggregate.featured_count, 0),
    album_count_denorm = COALESCE(aggregate.album_count, 0),
    aggregates_updated_at = now()
FROM (
    SELECT artist.id,
           aggregate.primary_count,
           aggregate.featured_count,
           aggregate.album_count
    FROM artists AS artist
    LEFT JOIN aggregate ON aggregate.artist_id = artist.id
    WHERE artist.merged_into IS NULL
) AS aggregate
WHERE artist.id = aggregate.id
  AND (
      artist.track_count_primary,
      artist.track_count_featured,
      artist.album_count_denorm
  ) IS DISTINCT FROM (
      COALESCE(aggregate.primary_count, 0),
      COALESCE(aggregate.featured_count, 0),
      COALESCE(aggregate.album_count, 0)
  )
