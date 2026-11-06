INSERT INTO chat.public_mutes (user_id, until, by, reason)
VALUES ($1, now() + $2::int * interval '1 second', $3, $4)
ON CONFLICT (user_id) DO UPDATE
SET until = EXCLUDED.until,
    by = EXCLUDED.by,
    reason = EXCLUDED.reason,
    created_at = now()
RETURNING until
