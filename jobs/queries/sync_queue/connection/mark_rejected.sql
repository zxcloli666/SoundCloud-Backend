UPDATE soundcloud_connections
SET refresh_failure_count = refresh_failure_count + 1,
    last_refresh_error_kind = 'token_rejected',
    last_refresh_error = 'SoundCloud rejected the refreshed access token',
    retry_at = GREATEST(retry_at, now() + $3::int * interval '1 second'),
    updated_at = now()
WHERE id = $1
  AND access_token = $2
  AND last_refresh_error_kind IS DISTINCT FROM 'token_rejected'
  AND last_refresh_error_kind IS DISTINCT FROM 'reauthorization_required'
  AND (refresh_lease_id IS NULL OR refresh_lease_expires_at <= now())
