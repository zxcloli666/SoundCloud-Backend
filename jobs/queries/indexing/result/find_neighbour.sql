WITH candidates AS MATERIALIZED (
    SELECT id
    FROM tracks
    WHERE audio_fingerprint IS NOT NULL
      AND substr(audio_fingerprint, 1, 64) = $1
      AND id <> $2
)
SELECT id, canonical_track_id
FROM tracks
WHERE id = (SELECT id FROM candidates ORDER BY id LIMIT 1)
  AND audio_fingerprint IS NOT NULL
  AND substr(audio_fingerprint, 1, 64) = $1
FOR UPDATE
