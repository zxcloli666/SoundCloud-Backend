WITH due AS (
    SELECT kind
    FROM background_schedules
    WHERE enabled = true
      AND next_run_at <= now()
    ORDER BY next_run_at, kind
    FOR UPDATE SKIP LOCKED
    LIMIT $1
), advanced AS (
    UPDATE background_schedules AS schedule
    SET next_run_at = now() + schedule.interval_seconds * interval '1 second',
        updated_at = now()
    FROM due
    WHERE schedule.kind = due.kind
    RETURNING schedule.kind, schedule.lane, schedule.priority, schedule.max_attempts
), commands AS MATERIALIZED (
    SELECT gen_random_uuid() AS id,
           advanced.kind,
           advanced.lane,
           advanced.priority,
           advanced.max_attempts
    FROM advanced
), accepted AS (
    INSERT INTO background_job_enqueues (id)
    SELECT id
    FROM commands
    ON CONFLICT (id) DO NOTHING
    RETURNING id
)
INSERT INTO background_jobs (
    id,
    kind,
    lane,
    dedup_key,
    payload,
    priority,
    max_attempts,
    available_at
)
SELECT commands.id,
       commands.kind,
       commands.lane,
       'schedule',
       '{"version":"1","payload":{}}'::jsonb,
       commands.priority,
       commands.max_attempts,
       now()
FROM commands
JOIN accepted USING (id)
ON CONFLICT (kind, dedup_key) WHERE dedup_key IS NOT NULL DO NOTHING
