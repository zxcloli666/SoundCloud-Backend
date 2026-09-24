UPDATE soundcloud_connections
SET refresh_lease_id = NULL,
    refresh_lease_expires_at = NULL,
    refresh_failure_count = refresh_failure_count + 1,
    refresh_rejection_count = 0,
    first_refresh_rejection_at = NULL,
    last_refresh_error_kind = $4,
    last_refresh_error = $5,
    retry_at = now() + $6::int * interval '1 second',
    updated_at = now()
WHERE id = $1
  AND refresh_lease_id = $2
  AND refresh_generation = $3
