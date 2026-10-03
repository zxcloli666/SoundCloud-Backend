UPDATE playlist_membership_state AS state
SET next_reconcile_at = clock_timestamp(),
    reconcile_failure_streak = 0,
    updated_at = clock_timestamp()
FROM playlists AS playlist
WHERE playlist.urn = state.playlist_urn
  AND playlist.deleted_at IS NULL
  AND (
      state.sync_status IN ('retry_wait', 'auth_required')
      OR state.conflict_code = 'catalog_incomplete'
  );
