UPDATE tracks
SET canonical_track_id = $1
WHERE id IN ($2, $3)
  AND canonical_track_id IS DISTINCT FROM $1
