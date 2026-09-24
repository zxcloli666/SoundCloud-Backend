UPDATE background_schedules
SET next_run_at = GREATEST(next_run_at, now() + $1::bigint * interval '1 millisecond'),
    updated_at = now()
WHERE kind = 'indexing.resolve_durations'
