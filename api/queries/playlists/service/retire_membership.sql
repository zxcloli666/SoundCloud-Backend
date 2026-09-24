UPDATE playlist_membership_state
SET next_reconcile_at = NULL, reconcile_generation = reconcile_generation + 1,
    updated_at = clock_timestamp()
WHERE playlist_urn = $1
