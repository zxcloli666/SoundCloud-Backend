INSERT INTO sync_queue (user_id, action_type, target_urn, payload)
VALUES ($1, $2, $3, $4)
ON CONFLICT (user_id, action_type, target_urn)
    WHERE action_type <> 'comment'
DO UPDATE SET
    payload = CASE WHEN sync_queue.action_type = 'track_update'
        THEN jsonb_build_object('track', COALESCE(sync_queue.payload->'track', '{}') || (EXCLUDED.payload->'track'))
        WHEN sync_queue.action_type = 'playlist_update'
        THEN jsonb_build_object('playlist', COALESCE(sync_queue.payload->'playlist', '{}') || (EXCLUDED.payload->'playlist'))
        ELSE EXCLUDED.payload END,
    generation = sync_queue.generation + 1,
    remote_attempted_generation = NULL,
    remote_completed_generation = NULL,
    remote_result = NULL,
    retry_count = 0,
    last_error = NULL,
    next_run_at = now(),
    dead = false,
    failed_at = NULL
