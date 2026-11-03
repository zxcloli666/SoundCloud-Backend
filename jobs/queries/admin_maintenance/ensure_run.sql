INSERT INTO admin_maintenance_runs (kind, run_id, status, phase)
VALUES ($1, $2, 'running', $3)
ON CONFLICT (kind) DO UPDATE
    SET run_id = EXCLUDED.run_id,
        status = 'running',
        phase = EXCLUDED.phase,
        cursor_uuid = NULL,
        cursor_text = NULL,
        scanned = 0,
        changed = 0,
        merged = 0,
        skipped = 0,
        started_at = now(),
        updated_at = now(),
        completed_at = NULL
    WHERE admin_maintenance_runs.status = 'completed'
      AND admin_maintenance_runs.run_id <> EXCLUDED.run_id
