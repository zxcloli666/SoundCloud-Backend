UPDATE tracks
SET audio_fingerprint = $2
WHERE id = $1
