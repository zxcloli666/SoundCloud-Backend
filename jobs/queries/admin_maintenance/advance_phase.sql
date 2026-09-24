UPDATE admin_maintenance_runs
SET phase = $3,
    cursor_uuid = NULL,
    cursor_text = NULL,
    updated_at = now()
WHERE kind = $1
  AND run_id = $2
  AND status = 'running'
