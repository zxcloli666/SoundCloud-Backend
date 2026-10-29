UPDATE tracks AS track
SET work_key                = keys.work_key,
    recording_key           = keys.recording_key,
    work_normalizer_version = $4
FROM UNNEST($1::uuid[], $2::text[], $3::text[]) AS keys(id, work_key, recording_key)
WHERE track.id = keys.id
  AND (track.work_key, track.recording_key, track.work_normalizer_version)
      IS DISTINCT FROM (keys.work_key, keys.recording_key, $4)
