UPDATE admin_maintenance_runs
SET status = 'completed',
    phase = 'done',
    cursor_uuid = NULL,
    cursor_text = NULL,
    completed_at = now(),
    updated_at = now()
WHERE kind = $1
  AND run_id = $2
  AND status = 'running'
