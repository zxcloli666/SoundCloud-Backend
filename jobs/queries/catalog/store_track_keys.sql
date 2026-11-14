WITH keys AS (SELECT keys.id, keys.work_key, keys.recording_key
              FROM UNNEST($1::uuid[], $2::text[], $3::text[]) AS keys(id, work_key, recording_key)),
     locked AS MATERIALIZED (SELECT track.id, keys.work_key, keys.recording_key
                             FROM tracks AS track
                                      JOIN keys ON keys.id = track.id
                             WHERE (track.work_key, track.recording_key, track.work_normalizer_version)
                                       IS DISTINCT FROM (keys.work_key, keys.recording_key, $4)
                             ORDER BY track.sc_track_id
                             FOR NO KEY UPDATE OF track),
     updated AS (
         UPDATE tracks AS track
             SET work_key = locked.work_key,
                 recording_key = locked.recording_key,
                 work_normalizer_version = $4
             FROM locked
             WHERE track.id = locked.id
             RETURNING track.id, track.primary_artist_id, track.superseded_by, locked.work_key, locked.recording_key)
INSERT INTO track_work_keys AS stored (track_id, primary_artist_id, superseded, work_key, recording_key,
                                       work_normalizer_version)
SELECT updated.id,
       updated.primary_artist_id,
       updated.superseded_by IS NOT NULL,
       updated.work_key,
       updated.recording_key,
       $4
FROM updated
ORDER BY updated.id
ON CONFLICT (track_id) DO UPDATE
    SET work_key = EXCLUDED.work_key,
        recording_key = EXCLUDED.recording_key,
        work_normalizer_version = EXCLUDED.work_normalizer_version
WHERE (stored.work_key, stored.recording_key, stored.work_normalizer_version)
          IS DISTINCT FROM (EXCLUDED.work_key, EXCLUDED.recording_key, EXCLUDED.work_normalizer_version)
