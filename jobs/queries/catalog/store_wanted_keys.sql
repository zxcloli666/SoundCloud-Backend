UPDATE wanted_tracks AS wanted
SET work_key                = keys.work_key,
    recording_key           = keys.recording_key,
    work_normalizer_version = $4
FROM UNNEST($1::uuid[], $2::text[], $3::text[]) AS keys(id, work_key, recording_key)
WHERE wanted.id = keys.id
