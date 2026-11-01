SELECT access_token,
       expires_at,
       scope,
       oauth_app_id,
       refresh_generation,
       refresh_failure_count,
       refresh_lease_id,
       refresh_lease_expires_at,
       last_refresh_attempt_at,
       last_refresh_success_at,
       last_refresh_error_kind,
       last_refresh_error,
       retry_at
FROM soundcloud_connections
WHERE id = $1
