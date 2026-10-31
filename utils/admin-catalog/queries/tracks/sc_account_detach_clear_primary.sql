WITH locked AS MATERIALIZED (SELECT track.id
                             FROM tracks track
                             WHERE track.uploader_sc_user_id = $2
                               AND track.primary_artist_id = $1
                             ORDER BY track.sc_track_id
                                 FOR NO KEY UPDATE OF track)
UPDATE tracks
SET primary_artist_id = NULL
FROM locked
WHERE tracks.id = locked.id
