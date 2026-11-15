WITH keys AS (SELECT keys.id, keys.work_key, keys.recording_key
              FROM UNNEST($1::uuid[], $2::text[], $3::text[]) AS keys(id, work_key, recording_key)),
     locked AS MATERIALIZED (SELECT stored.track_id, keys.work_key, keys.recording_key
                             FROM track_work_keys AS stored
                                      JOIN keys ON keys.id = stored.track_id
                                      JOIN tracks AS track ON track.id = stored.track_id
                             WHERE (stored.work_key, stored.recording_key, stored.work_normalizer_version)
                                       IS DISTINCT FROM (keys.work_key, keys.recording_key, $4)
                             ORDER BY track.sc_track_id
                             FOR NO KEY UPDATE OF stored)
UPDATE track_work_keys AS stored
SET work_key                = locked.work_key,
    recording_key           = locked.recording_key,
    work_normalizer_version = $4
FROM locked
WHERE stored.track_id = locked.track_id
