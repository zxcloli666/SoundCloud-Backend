SELECT primary_artist_id AS "primary_artist_id!",
       recording_key     AS "recording_key!"
FROM tracks
WHERE recording_key IS NOT NULL
  AND primary_artist_id IS NOT NULL
  AND superseded_by IS NULL
  AND ($1::uuid IS NULL OR (primary_artist_id, recording_key) > ($1::uuid, $2::text))
GROUP BY primary_artist_id, recording_key
HAVING count(*) > 1
ORDER BY primary_artist_id, recording_key
LIMIT $3
