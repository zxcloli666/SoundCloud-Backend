UPDATE tracks
SET needs_duration_resolve = false,
    duration_resolve_attempts = 0,
    duration_resolve_retry_at = NULL,
    updated_at = now()
WHERE sc_track_id = $1
  AND needs_duration_resolve = true
