WITH by_artist AS (
    SELECT track.id
    FROM tracks AS track
    WHERE track.primary_artist_id = $1
    UNION
    SELECT credit.track_id
    FROM track_artists AS credit
    WHERE credit.artist_id = $1
      AND credit.role = 'primary'
)
SELECT track.id,
       track.sc_track_id,
       COALESCE(track.title, '') AS "title!"
FROM by_artist
JOIN tracks AS track ON track.id = by_artist.id
WHERE track.superseded_by IS NULL
  AND (track.work_key = ANY ($2)
    OR track.title_normalized = $3
    OR track.title_normalized LIKE $4
    OR EXISTS (SELECT 1
               FROM track_work_aliases AS alias
               WHERE alias.track_id = track.id
                 AND alias.alias_key = ANY ($2)))
LIMIT $5
