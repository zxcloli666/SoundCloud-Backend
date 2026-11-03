WITH locked AS MATERIALIZED (
    SELECT state.track_id,
           state.priority,
           state.next_run_at,
           state.created_at,
           CASE
               WHEN state.wake_message_id IS NOT NULL AND state.wake_durable_at IS NULL THEN state.updated_at
           END AS wake_at
    FROM lyrics_lookup_state AS state
    WHERE state.track_id = ANY ($1::uuid[])
    ORDER BY state.track_id
    FOR SHARE OF state SKIP LOCKED
)
INSERT INTO lyrics_lookup_queue AS queue (track_id, priority, next_run_at, created_at, wake_at)
SELECT locked.track_id,
       locked.priority,
       locked.next_run_at,
       locked.created_at,
       locked.wake_at
FROM locked
ON CONFLICT (track_id) DO UPDATE
SET priority = EXCLUDED.priority,
    next_run_at = EXCLUDED.next_run_at,
    created_at = EXCLUDED.created_at,
    wake_at = EXCLUDED.wake_at
WHERE (queue.priority, queue.next_run_at, queue.created_at, queue.wake_at)
    IS DISTINCT FROM (EXCLUDED.priority, EXCLUDED.next_run_at, EXCLUDED.created_at, EXCLUDED.wake_at)
