SELECT run_id
FROM admin_maintenance_runs
WHERE kind = $1
  AND status = 'running'
