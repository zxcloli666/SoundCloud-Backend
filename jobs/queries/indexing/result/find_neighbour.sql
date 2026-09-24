SELECT id, canonical_track_id
FROM tracks
WHERE substr(audio_fingerprint, 1, 64) = $1
  AND id <> $2
ORDER BY id
LIMIT 1
FOR UPDATE
