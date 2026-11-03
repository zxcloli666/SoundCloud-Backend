UPDATE sync_queue AS queued
SET exec_generation = queued.generation,
    retry_count = 0,
    last_error = NULL,
    next_run_at = now(),
    dead = false,
    failed_at = NULL,
    remote_completed_generation = NULL,
    remote_result = NULL
FROM (
    SELECT id
    FROM sync_queue
    WHERE exec_generation IS DISTINCT FROM generation
      AND (exec_generation IS NOT NULL OR intent_v > 0)
    ORDER BY id
    LIMIT $1
    FOR UPDATE SKIP LOCKED
) AS fresh
WHERE queued.id = fresh.id
