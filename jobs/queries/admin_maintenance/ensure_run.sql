INSERT INTO admin_maintenance_runs (kind, run_id, status, phase)
VALUES ($1, $2, 'running', $3)
ON CONFLICT (kind) DO NOTHING
