UPDATE admin_maintenance_runs
SET scanned = $3,
    changed = $4,
    merged = $5,
    skipped = $6,
    updated_at = now()
WHERE kind = $1
  AND run_id = $2
  AND status = 'running'
