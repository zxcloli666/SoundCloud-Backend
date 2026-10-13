INSERT INTO sync_queue_action_pauses (oauth_app_id, action_type, paused_until, strikes)
VALUES ($1, $2, now() + $3::bigint * interval '1 second', $4)
ON CONFLICT (oauth_app_id, action_type) DO UPDATE
SET paused_until = EXCLUDED.paused_until,
    strikes = EXCLUDED.strikes,
    updated_at = now()
