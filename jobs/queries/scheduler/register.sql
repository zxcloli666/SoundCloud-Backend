INSERT INTO background_schedules (
    kind,
    lane,
    interval_seconds,
    next_run_at,
    priority,
    max_attempts,
    enabled
)
VALUES ($1, $2, $3, now() + ($7::integer * interval '1 second'), $4, $5, COALESCE($6, true))
ON CONFLICT (kind) DO UPDATE
SET lane = excluded.lane,
    interval_seconds = excluded.interval_seconds,
    priority = excluded.priority,
    max_attempts = excluded.max_attempts,
    enabled = COALESCE($6, background_schedules.enabled),
    updated_at = now()
