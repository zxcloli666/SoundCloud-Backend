UPDATE tracks
SET canonical_track_id = canonical_track_id
WHERE id = $1
