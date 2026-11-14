UPDATE tracks
SET sc_write_confirmed = true
WHERE sc_track_id = $1 AND deleted_at IS NOT NULL
