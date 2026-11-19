INSERT INTO llm_pause (ladder, reason, paused_until)
VALUES ($1, $2, now() + $3::float8 * interval '1 second')
ON CONFLICT (ladder) DO UPDATE
SET reason = EXCLUDED.reason,
    paused_until = EXCLUDED.paused_until,
    updated_at = now()
RETURNING alerted_at IS NOT NULL AS "alerted!"
