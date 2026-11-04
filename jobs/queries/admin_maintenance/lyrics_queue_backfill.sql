WITH batch AS MATERIALIZED (
    SELECT state.track_id,
           state.priority,
           state.next_run_at,
           state.created_at,
           CASE
               WHEN state.wake_message_id IS NOT NULL AND state.wake_durable_at IS NULL THEN state.updated_at
           END AS wake_at
    FROM lyrics_lookup_state AS state
    WHERE state.ctid >= $1::text::tid
      AND state.ctid < $2::text::tid
    FOR SHARE OF state SKIP LOCKED
), inserted AS (
    INSERT INTO lyrics_lookup_queue (track_id, priority, next_run_at, created_at, wake_at)
    SELECT batch.track_id,
           batch.priority,
           batch.next_run_at,
           batch.created_at,
           batch.wake_at
    FROM batch
    ORDER BY batch.track_id
    ON CONFLICT (track_id) DO NOTHING
    RETURNING track_id
)
SELECT (SELECT count(*) FROM batch) AS "scanned!",
       (SELECT count(*) FROM inserted) AS "inserted!"
