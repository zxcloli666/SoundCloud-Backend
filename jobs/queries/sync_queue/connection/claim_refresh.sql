UPDATE soundcloud_connections
SET refresh_lease_id = $4,
    refresh_lease_expires_at = now() + $5::int * interval '1 second',
    refresh_generation = refresh_generation + 1,
    last_refresh_attempt_at = now(),
    updated_at = now()
WHERE id = $1
  AND refresh_generation = $2
  AND access_token = $3
  AND (refresh_lease_id IS NULL OR refresh_lease_expires_at <= now())
  AND last_refresh_error_kind IS DISTINCT FROM 'reauthorization_required'
  AND (retry_at IS NULL OR retry_at <= now())
RETURNING refresh_generation
