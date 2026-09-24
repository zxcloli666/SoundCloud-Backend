SELECT run_id,
       status,
       phase,
       cursor_uuid,
       cursor_text,
       scanned,
       changed,
       merged,
       skipped
FROM admin_maintenance_runs
WHERE kind = $1
