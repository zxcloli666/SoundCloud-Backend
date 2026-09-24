UPDATE playlist_membership_state AS state
SET next_reconcile_at = clock_timestamp(),
    updated_at = clock_timestamp()
FROM (
    SELECT DISTINCT intent.playlist_urn
    FROM playlist_legacy_membership_intents AS intent
    JOIN playlist_membership_state AS due
      ON due.playlist_urn = intent.playlist_urn
    WHERE intent.classification = 'unclassified'
      AND intent.resolved_at IS NULL
      AND (due.next_reconcile_at IS NULL OR due.next_reconcile_at > clock_timestamp())
    ORDER BY intent.playlist_urn
    LIMIT $1
) AS pending
WHERE state.playlist_urn = pending.playlist_urn
