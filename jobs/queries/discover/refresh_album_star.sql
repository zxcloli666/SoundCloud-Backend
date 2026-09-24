WITH aggregate AS (
    SELECT album.id,
           COALESCE(artist.is_star, false) AS is_star_artist
    FROM albums AS album
    LEFT JOIN artists AS artist ON artist.id = album.primary_artist_id
)
UPDATE albums AS album
SET is_star_artist = aggregate.is_star_artist
FROM aggregate
WHERE album.id = aggregate.id
  AND album.is_star_artist IS DISTINCT FROM aggregate.is_star_artist
