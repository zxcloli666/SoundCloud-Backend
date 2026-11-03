WITH keys AS (SELECT keys.id, keys.work_key, keys.recording_key
              FROM UNNEST($1::uuid[], $2::text[], $3::text[]) AS keys(id, work_key, recording_key)),
     locked AS MATERIALIZED (SELECT track.id, keys.work_key, keys.recording_key
                             FROM tracks AS track
                                      JOIN keys ON keys.id = track.id
                             WHERE (track.work_key, track.recording_key, track.work_normalizer_version)
                                       IS DISTINCT FROM (keys.work_key, keys.recording_key, $4)
                             ORDER BY track.sc_track_id
                             FOR NO KEY UPDATE OF track)
UPDATE tracks AS track
SET work_key                = locked.work_key,
    recording_key           = locked.recording_key,
    work_normalizer_version = $4
FROM locked
WHERE track.id = locked.id
