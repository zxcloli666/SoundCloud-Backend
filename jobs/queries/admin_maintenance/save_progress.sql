UPDATE admin_maintenance_runs
SET phase = $3,
    cursor_uuid = $4,
    cursor_text = $5,
    scanned = scanned + $6,
    changed = changed + $7,
    merged = merged + $8,
    skipped = skipped + $9,
    updated_at = now()
WHERE kind = $1
  AND run_id = $2
  AND status = 'running'
