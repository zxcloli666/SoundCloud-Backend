SELECT state.next_reconcile_at AS "due_at!"
FROM playlist_membership_state AS state
WHERE state.playlist_urn = $1
  AND EXISTS(SELECT 1 FROM playlists p WHERE p.urn = state.playlist_urn AND p.deleted_at IS NULL)
  AND state.next_reconcile_at <= $2
