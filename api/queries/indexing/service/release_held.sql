UPDATE tracks
SET pipeline_held = false,
    updated_at = now()
WHERE sc_track_id = $1
  AND pipeline_held
  AND deleted_at IS NULL
RETURNING duration_ms
