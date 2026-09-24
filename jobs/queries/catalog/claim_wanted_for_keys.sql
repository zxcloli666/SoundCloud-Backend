SELECT id, title
FROM wanted_tracks
WHERE work_normalizer_version IS NULL
ORDER BY id
LIMIT $1
