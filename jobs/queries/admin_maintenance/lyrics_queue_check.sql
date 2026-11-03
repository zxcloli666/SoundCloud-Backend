WITH expected AS MATERIALIZED (
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
), stored AS MATERIALIZED (
    SELECT queue.track_id,
           queue.priority,
           queue.next_run_at,
           queue.created_at,
           queue.wake_at
    FROM lyrics_lookup_queue AS queue
    WHERE queue.track_id = ANY (ARRAY(SELECT expected.track_id FROM expected))
)
SELECT (SELECT count(*) FROM expected) AS "scanned!",
       ARRAY(
           SELECT expected.track_id
           FROM expected
           LEFT JOIN stored ON stored.track_id = expected.track_id
           WHERE stored.track_id IS NULL
              OR (expected.priority, expected.next_run_at, expected.created_at, expected.wake_at)
                  IS DISTINCT FROM (stored.priority, stored.next_run_at, stored.created_at, stored.wake_at)
           ORDER BY expected.track_id
       ) AS "mismatched!"
