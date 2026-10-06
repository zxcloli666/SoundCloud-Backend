UPDATE playlist_membership_state AS state
SET next_reconcile_at = clock_timestamp(),
    reconcile_failure_streak = 0,
    updated_at = clock_timestamp()
FROM playlists AS playlist
WHERE playlist.urn = state.playlist_urn
  AND playlist.deleted_at IS NULL
  AND (
      state.sync_status = 'retry_wait'
      OR state.conflict_code = 'catalog_incomplete'
      OR (
          state.sync_status = 'auth_required'
          AND playlist.sharing = 'public'
          AND playlist.last_read_at >= now() - interval '7 days'
      )
  );
