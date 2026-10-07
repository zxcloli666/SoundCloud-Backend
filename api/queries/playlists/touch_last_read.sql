WITH touched AS (
    UPDATE playlists
    SET last_read_at = now()
    WHERE urn = $1
      AND (last_read_at IS NULL OR last_read_at < now() - INTERVAL '6 hours')
    RETURNING urn, sharing
)
UPDATE playlist_membership_state AS state
SET sync_status = 'retry_wait',
    next_reconcile_at = clock_timestamp(),
    reconcile_failure_streak = 0,
    updated_at = clock_timestamp()
FROM touched
WHERE state.playlist_urn = touched.urn
  AND touched.sharing = 'public'
  AND state.sync_status = 'auth_required'
