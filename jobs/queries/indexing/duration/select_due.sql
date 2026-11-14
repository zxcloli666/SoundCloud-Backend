SELECT track.sc_track_id
FROM tracks AS track
LEFT JOIN track_sync_stamps AS stamp ON stamp.track_id = track.id
WHERE track.needs_duration_resolve = true
  AND (track.duration_resolve_retry_at IS NULL OR track.duration_resolve_retry_at <= now())
ORDER BY COALESCE(track.duration_resolve_retry_at, GREATEST(track.sc_synced_at, stamp.sc_synced_at)), track.sc_track_id
LIMIT $1
