INSERT INTO sync_queue (id, user_id, action_type, target_urn, payload, generation, cancelled)
VALUES (public.scd_uuid('sync_queue', $1, $2, $3), $1, $2, $3, $4, public.scd_hlc(0), $5)
ON CONFLICT (user_id, action_type, target_urn)
    WHERE action_type <> 'comment'
DO UPDATE SET
    payload = CASE WHEN sync_queue.cancelled THEN EXCLUDED.payload
        WHEN sync_queue.action_type = 'track_update'
        THEN jsonb_build_object('track', COALESCE(sync_queue.payload->'track', '{}') || (EXCLUDED.payload->'track'))
        WHEN sync_queue.action_type = 'playlist_update'
        THEN jsonb_build_object('playlist', COALESCE(sync_queue.payload->'playlist', '{}') || (EXCLUDED.payload->'playlist'))
        ELSE EXCLUDED.payload END,
    generation = public.scd_hlc(sync_queue.generation),
    cancelled = EXCLUDED.cancelled,
    created_at = CASE WHEN sync_queue.cancelled THEN now() ELSE sync_queue.created_at END
WHERE NOT EXCLUDED.cancelled
   OR sync_queue.cancelled
