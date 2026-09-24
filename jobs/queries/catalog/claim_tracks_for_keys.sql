SELECT id, title
FROM tracks
WHERE work_normalizer_version IS NULL
ORDER BY id
LIMIT $1
