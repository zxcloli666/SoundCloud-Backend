UPDATE taste_schedule
SET next_refresh_at = now() + $1::bigint * interval '1 second'
WHERE singleton
  AND next_refresh_at <= now()
RETURNING (now() AT TIME ZONE 'UTC') AS "now!"
