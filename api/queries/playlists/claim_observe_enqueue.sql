UPDATE playlist_membership_state AS state
SET next_reconcile_at = clock_timestamp() + interval '30 seconds',
    updated_at = clock_timestamp()
WHERE state.playlist_urn = $1
  AND EXISTS(SELECT 1 FROM playlists p WHERE p.urn = state.playlist_urn AND p.deleted_at IS NULL)
  AND state.next_reconcile_at <= clock_timestamp()
RETURNING state.next_reconcile_at AS "claimed_until!"
