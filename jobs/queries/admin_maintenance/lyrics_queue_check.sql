WITH expected AS MATERIALIZED (
    SELECT state.track_id,
           state.priority,
           state.next_run_at,
           state.created_at,
           CASE
               WHEN state.wake_message_id IS NOT NULL AND state.wake_durable_at IS NULL THEN state.updated_at
           END AS wake_at,
           state.attempts,
           state.last_attempt_at,
           state.claim_job_id,
           state.claim_job_generation,
           state.claim_job_lease_id,
           state.claim_state_generation,
           state.claim_expires_at
    FROM lyrics_lookup_state AS state
    WHERE state.ctid >= $1::text::tid
      AND state.ctid < $2::text::tid
), stored AS MATERIALIZED (
    SELECT queue.track_id,
           queue.priority,
           queue.next_run_at,
           queue.created_at,
           queue.wake_at,
           queue.attempts,
           queue.last_attempt_at,
           queue.claim_job_id,
           queue.claim_job_generation,
           queue.claim_job_lease_id,
           queue.claim_state_generation,
           queue.claim_expires_at
    FROM lyrics_lookup_queue AS queue
    WHERE queue.track_id = ANY (ARRAY(SELECT expected.track_id FROM expected))
)
SELECT (SELECT count(*) FROM expected) AS "scanned!",
       ARRAY(
           SELECT expected.track_id
           FROM expected
           LEFT JOIN stored ON stored.track_id = expected.track_id
           WHERE stored.track_id IS NULL
              OR (
                  expected.priority,
                  expected.next_run_at,
                  expected.created_at,
                  expected.wake_at,
                  expected.attempts,
                  expected.last_attempt_at,
                  expected.claim_job_id,
                  expected.claim_job_generation,
                  expected.claim_job_lease_id,
                  expected.claim_state_generation,
                  expected.claim_expires_at
              ) IS DISTINCT FROM (
                  stored.priority,
                  stored.next_run_at,
                  stored.created_at,
                  stored.wake_at,
                  stored.attempts,
                  stored.last_attempt_at,
                  stored.claim_job_id,
                  stored.claim_job_generation,
                  stored.claim_job_lease_id,
                  stored.claim_state_generation,
                  stored.claim_expires_at
              )
           ORDER BY expected.track_id
       ) AS "mismatched!"
