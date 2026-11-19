INSERT INTO video_gen_pause (id, reason, paused_until)
VALUES (1, $1, now() + $2::float8 * interval '1 second')
ON CONFLICT (id) DO UPDATE
SET reason = EXCLUDED.reason,
    paused_until = EXCLUDED.paused_until,
    updated_at = now()
RETURNING alerted_at IS NOT NULL AS "alerted!"
