UPDATE playlist_membership_state AS state
SET next_reconcile_at = clock_timestamp(),
    updated_at = clock_timestamp()
WHERE state.playlist_urn = $1
  AND NOT EXISTS (
      SELECT 1
      FROM playlist_legacy_membership_intents AS intent
      WHERE intent.playlist_urn = state.playlist_urn
        AND intent.classification NOT IN ('resolved', 'abandoned')
  )
