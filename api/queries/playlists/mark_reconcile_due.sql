UPDATE playlist_membership_state
SET next_reconcile_at = clock_timestamp(),
    updated_at = clock_timestamp()
WHERE playlist_urn = $1
  AND (next_reconcile_at IS NULL OR next_reconcile_at > clock_timestamp())
