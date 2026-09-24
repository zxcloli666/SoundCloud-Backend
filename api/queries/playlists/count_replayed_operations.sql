SELECT count(*) AS "count!"
FROM playlist_membership_operations
WHERE playlist_urn = $1
  AND idempotency_key = ANY ($2::uuid[])
