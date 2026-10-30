WITH evidence AS (
    SELECT id,
           refresh_rejection_count >= 2
               AND first_refresh_rejection_at <= now() - interval '15 minutes'
               AND COALESCE(last_refresh_success_at, created_at) <= now() - interval '30 minutes'
               AND expires_at <= now() AS confirmed
    FROM soundcloud_connections
    WHERE id = $1
      AND refresh_lease_id = $2
      AND refresh_generation = $3
      AND refresh_lease_expires_at > now()
    FOR UPDATE
)
UPDATE soundcloud_connections AS connection
SET refresh_lease_id = NULL,
    refresh_lease_expires_at = NULL,
    refresh_failure_count = LEAST(refresh_failure_count, 2147483646) + 1,
    refresh_rejection_count = LEAST(refresh_rejection_count, 2147483646) + 1,
    first_refresh_rejection_at = COALESCE(first_refresh_rejection_at, now()),
    last_refresh_error_kind = CASE WHEN evidence.confirmed
        THEN 'reauthorization_required' ELSE 'temporarily_unavailable' END,
    last_refresh_error = $4,
    retry_at = CASE WHEN evidence.confirmed THEN NULL ELSE now() + interval '10 minutes' END,
    updated_at = now()
FROM evidence
WHERE connection.id = evidence.id
