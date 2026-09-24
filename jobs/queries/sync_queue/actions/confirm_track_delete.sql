UPDATE tracks
SET sc_write_confirmed = true,
    updated_at = clock_timestamp()
WHERE sc_track_id = $1 AND deleted_at IS NOT NULL
