SELECT wanted.id,
       wanted.title,
       COALESCE(artist.name, '') AS "artist_name!",
       wanted.duration_ms,
       wanted.isrc,
       wanted.primary_artist_id
FROM wanted_tracks AS wanted
         LEFT JOIN artists AS artist ON artist.id = wanted.primary_artist_id
WHERE wanted.id = ANY ($1)
  AND wanted.title <> ''
