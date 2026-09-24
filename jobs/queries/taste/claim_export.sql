UPDATE taste_schedule
SET next_export_at = now() + $1::bigint * interval '1 second'
WHERE singleton
  AND next_export_at <= now()
RETURNING true AS "claimed!"
