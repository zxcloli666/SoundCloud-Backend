WITH locked AS MATERIALIZED (SELECT track.id
                             FROM tracks track
                             WHERE track.primary_artist_id = $2
                             ORDER BY track.sc_track_id
                                 FOR NO KEY UPDATE OF track)
UPDATE tracks
SET primary_artist_id = $1
FROM locked
WHERE tracks.id = locked.id
