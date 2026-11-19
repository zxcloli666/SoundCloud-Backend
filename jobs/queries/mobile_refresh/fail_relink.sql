UPDATE mobile_credentials
SET status = 'relink_required',
    refresh_lease_id = NULL,
    refresh_lease_expires_at = NULL,
    refresh_failure_count = LEAST(refresh_failure_count, 2147483646) + 1,
    last_refresh_error_kind = 'relink_required',
    last_refresh_error = $4,
    retry_at = NULL,
    updated_at = now()
WHERE soundcloud_user_id = $1
  AND refresh_lease_id = $2
  AND refresh_generation = $3
