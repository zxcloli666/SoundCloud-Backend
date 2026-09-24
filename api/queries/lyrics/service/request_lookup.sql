WITH locked AS MATERIALIZED (
    SELECT state.track_id,
           state.status,
           state.generation,
           state.next_run_at,
           state.failure_streak,
           state.wake_message_id,
           state.wake_generation,
           state.wake_durable_at,
           state.claim_job_id,
           state.last_attempt_at,
           state.miss_streak
    FROM lyrics_lookup_state AS state
    JOIN tracks AS track ON track.id = state.track_id
    WHERE track.sc_track_id = $1
    FOR UPDATE OF state
), decision AS (
    SELECT locked.*,
           locked.claim_job_id IS NULL
               AND locked.status IN ('retry', 'not_found')
               AND locked.next_run_at <= now() AS reopen,
           locked.claim_job_id IS NULL
               AND locked.status = 'pending'
               AND locked.wake_message_id IS NULL AS repair_wake
    FROM locked
), updated AS (
    UPDATE lyrics_lookup_state AS state
    SET status = CASE WHEN decision.reopen THEN 'pending' ELSE state.status END,
        generation = CASE WHEN decision.reopen THEN state.generation + 1 ELSE state.generation END,
        priority = 0,
        next_run_at = CASE WHEN decision.reopen THEN now() ELSE state.next_run_at END,
        last_outcome = CASE WHEN decision.reopen THEN NULL ELSE state.last_outcome END,
        last_error = CASE WHEN decision.reopen THEN NULL ELSE state.last_error END,
        retry_after_at = CASE WHEN decision.reopen THEN NULL ELSE state.retry_after_at END,
        wake_message_id = CASE
            WHEN decision.reopen OR decision.repair_wake THEN gen_random_uuid()
            ELSE state.wake_message_id
        END,
        wake_generation = CASE
            WHEN decision.reopen THEN state.generation + 1
            WHEN decision.repair_wake THEN state.generation
            ELSE state.wake_generation
        END,
        wake_durable_at = CASE
            WHEN decision.reopen OR decision.repair_wake THEN NULL
            ELSE state.wake_durable_at
        END,
        updated_at = now()
    FROM decision
    WHERE state.track_id = decision.track_id
    RETURNING state.status,
              state.generation,
              state.next_run_at,
              state.failure_streak,
              state.wake_message_id,
              state.wake_generation,
              state.wake_durable_at,
              state.claim_job_id,
              state.miss_streak
)
SELECT status,
       generation,
       next_run_at,
       failure_streak,
       wake_message_id,
       wake_generation,
       wake_durable_at,
       claim_job_id,
       miss_streak
FROM updated
