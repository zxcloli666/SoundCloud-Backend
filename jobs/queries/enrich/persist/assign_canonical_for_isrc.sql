WITH locked AS MATERIALIZED (SELECT track.id,
                                    track.isrc = $2 AND track.canonical_track_id IS NULL AS unassigned
                             FROM tracks AS track
                             WHERE (track.isrc = $2 AND track.canonical_track_id IS NULL)
                                OR track.id = $3
                             ORDER BY track.sc_track_id
                             FOR NO KEY UPDATE OF track)
UPDATE tracks AS track
SET canonical_track_id = $1
FROM locked
WHERE track.id = locked.id
  AND locked.unassigned
