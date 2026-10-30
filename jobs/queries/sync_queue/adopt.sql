UPDATE sync_queue
SET exec_generation = generation,
    retry_count = 0,
    last_error = NULL,
    next_run_at = now(),
    dead = false,
    failed_at = NULL,
    remote_attempted_generation = NULL,
    remote_completed_generation = NULL,
    remote_result = NULL
WHERE id IN (
    SELECT id
    FROM sync_queue
    WHERE exec_generation IS DISTINCT FROM generation
      AND (exec_generation IS NOT NULL OR intent_v > 0)
    LIMIT $1
    FOR UPDATE SKIP LOCKED
)
