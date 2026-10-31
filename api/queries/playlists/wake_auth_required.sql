UPDATE playlist_membership_state AS state
SET sync_status = 'retry_wait',
    next_reconcile_at = clock_timestamp(),
    reconcile_failure_streak = 0,
    updated_at = clock_timestamp()
FROM playlists AS playlist
WHERE state.playlist_urn = $1
  AND playlist.urn = state.playlist_urn
  AND playlist.sharing = 'public'
  AND state.sync_status = 'auth_required'
  AND (playlist.last_read_at IS NULL OR playlist.last_read_at < now() - INTERVAL '6 hours')
