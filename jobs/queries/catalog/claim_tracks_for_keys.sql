SELECT track.id, track.title
FROM track_work_keys AS keys
JOIN tracks AS track ON track.id = keys.track_id
WHERE keys.work_normalizer_version IS NULL
ORDER BY keys.track_id
LIMIT $1
