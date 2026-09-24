SELECT track.id,
       track.sc_track_id,
       COALESCE(track.title, '') AS "title!"
FROM tracks AS track
WHERE track.superseded_by IS NULL
  AND (track.primary_artist_id = $1
    OR EXISTS (SELECT 1
               FROM track_artists AS credit
               WHERE credit.track_id = track.id
                 AND credit.artist_id = $1
                 AND credit.role = 'primary'))
  AND (track.work_key = ANY ($2)
    OR track.title_normalized = $3
    OR track.title_normalized LIKE $4
    OR EXISTS (SELECT 1
               FROM track_work_aliases AS alias
               WHERE alias.track_id = track.id
                 AND alias.alias_key = ANY ($2)))
LIMIT $5
