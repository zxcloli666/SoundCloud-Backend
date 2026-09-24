SELECT kind,
       run_id,
       status,
       phase,
       scanned,
       changed,
       merged,
       skipped,
       started_at,
       updated_at,
       completed_at
FROM admin_maintenance_runs
ORDER BY kind
