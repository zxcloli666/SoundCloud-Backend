UPDATE background_jobs
SET lease_id = NULL,
    lease_generation = NULL,
    leased_by = NULL,
    lease_expires_at = NULL,
    attempts = GREATEST(attempts - 1, 0),
    priority = CASE
        WHEN $7 AND priority >= $8::smallint THEN priority - $9::smallint
        WHEN priority > $6::smallint THEN priority - 1
        ELSE priority
    END,
    available_at = now() + $4::bigint * interval '1 millisecond',
    last_error = $5,
    updated_at = now()
WHERE id = $1
  AND lease_id = $2
  AND lease_generation = $3
  AND generation = $3
  AND lease_expires_at > now()
