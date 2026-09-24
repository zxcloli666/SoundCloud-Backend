SELECT sc_track_id
FROM tracks
WHERE needs_duration_resolve = true
  AND (duration_resolve_retry_at IS NULL OR duration_resolve_retry_at <= now())
ORDER BY COALESCE(duration_resolve_retry_at, sc_synced_at), sc_track_id
LIMIT $1
