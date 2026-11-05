SELECT id,
       COALESCE(rotation_seed, '') AS "seed!",
       COALESCE(refresh_hash, '') AS "refresh_hash!",
       COALESCE(rotated_at > now() - ($3::int * INTERVAL '1 second'), false) AS "fresh!"
FROM remote_grants
WHERE prev_refresh_hash = $1
  AND client_id = $2
  AND refresh_expires_at > now()
LIMIT 1
