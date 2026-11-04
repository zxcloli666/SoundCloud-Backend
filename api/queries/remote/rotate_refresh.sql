UPDATE remote_grants
SET prev_refresh_hash = refresh_hash,
    rotated_at = now(),
    rotation_seed = $7,
    access_hash = $2,
    access_expires_at = now() + ($3::int * INTERVAL '1 second'),
    refresh_hash = $4,
    refresh_expires_at = now() + ($5::int * INTERVAL '1 second'),
    last_used_at = now()
WHERE refresh_hash = $1
  AND client_id = $6
  AND refresh_expires_at > now()
RETURNING id
