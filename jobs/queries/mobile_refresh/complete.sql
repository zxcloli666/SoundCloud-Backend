UPDATE mobile_credentials
SET access_token = $4,
    refresh_token = COALESCE($5, refresh_token),
    expires_at = $6,
    scope = COALESCE($7, scope),
    refresh_generation = refresh_generation + 1,
    refresh_failure_count = 0,
    refresh_lease_id = NULL,
    refresh_lease_expires_at = NULL,
    last_refresh_success_at = now(),
    last_refresh_error_kind = NULL,
    last_refresh_error = NULL,
    retry_at = NULL,
    updated_at = now()
WHERE soundcloud_user_id = $1
  AND refresh_lease_id = $2
  AND refresh_generation = $3
RETURNING refresh_generation
