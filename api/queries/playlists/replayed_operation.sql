SELECT outcome, conflict_code
FROM playlist_membership_operations
WHERE playlist_urn = $1
  AND idempotency_key = $2
