WITH locked AS MATERIALIZED (SELECT track.id
                             FROM tracks AS track
                             WHERE track.primary_artist_id = $1
                                OR track.cover_of_artist_id = $1
                             ORDER BY track.sc_track_id
                             FOR NO KEY UPDATE OF track)
UPDATE tracks AS track
SET primary_artist_id  = CASE WHEN track.primary_artist_id = $1 THEN $2 ELSE track.primary_artist_id END,
    cover_of_artist_id = CASE WHEN track.cover_of_artist_id = $1 THEN $2 ELSE track.cover_of_artist_id END
FROM locked
WHERE track.id = locked.id
