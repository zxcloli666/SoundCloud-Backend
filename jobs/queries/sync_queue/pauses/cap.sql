UPDATE sync_queue_action_pauses
SET paused_until = now() + $1::bigint * interval '1 second',
    updated_at = now()
WHERE paused_until > now() + $1::bigint * interval '1 second'
