INSERT INTO soundcloud_connections (
    id,
    soundcloud_user_id,
    username,
    oauth_app_id,
    access_token,
    refresh_token,
    expires_at,
    scope,
    last_refresh_success_at
)
VALUES (
    $1,
    $2,
    NULLIF($3, ''),
    $4,
    $5,
    $6,
    $7,
    $8,
    now()
)
RETURNING id,
          soundcloud_user_id,
          oauth_app_id,
          access_token,
          refresh_token,
          expires_at,
          scope,
          refresh_generation,
          refresh_failure_count,
          refresh_lease_id,
          refresh_lease_expires_at,
          last_refresh_attempt_at,
          last_refresh_success_at,
          last_refresh_error_kind,
          last_refresh_error,
          retry_at
