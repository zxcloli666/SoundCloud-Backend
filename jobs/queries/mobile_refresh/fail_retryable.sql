UPDATE mobile_credentials
SET refresh_lease_id = NULL,
    refresh_lease_expires_at = NULL,
    refresh_failure_count = LEAST(refresh_failure_count, 2147483646) + 1,
    last_refresh_error_kind = $4,
    last_refresh_error = $5,
    retry_at = now() + $6::int * interval '1 second',
    updated_at = now()
WHERE soundcloud_user_id = $1
  AND refresh_lease_id = $2
  AND refresh_generation = $3
