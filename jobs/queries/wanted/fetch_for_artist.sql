WITH picked AS (
    SELECT wanted.id,
           row_number() OVER (
               ORDER BY wanted.fetched_at NULLS FIRST, wanted.updated_at NULLS FIRST
           ) AS position
    FROM wanted_tracks AS wanted
    WHERE wanted.status = 'wanted'
      AND wanted.track_id IS NULL
      AND wanted.title <> ''
      AND wanted.primary_artist_id = $1
    ORDER BY wanted.fetched_at NULLS FIRST, wanted.updated_at NULLS FIRST
    LIMIT $2
), stamped AS (
    UPDATE wanted_tracks AS wanted
    SET fetched_at = now()
    FROM picked
    WHERE wanted.id = picked.id
    RETURNING wanted.id, wanted.title, wanted.duration_ms, wanted.isrc, wanted.primary_artist_id
)
SELECT stamped.id,
       stamped.title,
       COALESCE(artist.name, '') AS "artist_name!",
       stamped.duration_ms,
       stamped.isrc,
       stamped.primary_artist_id
FROM stamped
         JOIN picked ON picked.id = stamped.id
         LEFT JOIN artists AS artist ON artist.id = stamped.primary_artist_id
ORDER BY picked.position
