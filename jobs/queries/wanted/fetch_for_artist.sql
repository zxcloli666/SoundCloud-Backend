SELECT wanted.id,
       wanted.title,
       COALESCE(artist.name, '') AS "artist_name!",
       wanted.duration_ms,
       wanted.isrc,
       wanted.primary_artist_id
FROM wanted_tracks AS wanted
         LEFT JOIN artists AS artist ON artist.id = wanted.primary_artist_id
WHERE wanted.status = 'wanted'
  AND wanted.track_id IS NULL
  AND wanted.title <> ''
  AND wanted.primary_artist_id = $1
ORDER BY wanted.updated_at NULLS FIRST
LIMIT $2
