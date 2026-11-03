WITH candidates AS MATERIALIZED (
    SELECT neighbour.id, neighbour.canonical_track_id
    FROM tracks AS neighbour
    WHERE neighbour.audio_fingerprint IS NOT NULL
      AND substr(neighbour.audio_fingerprint, 1, 64) = $1
      AND neighbour.sc_track_id <> $2
)
SELECT id, canonical_track_id
FROM candidates
ORDER BY id
LIMIT 1
