SELECT id,
       user_id,
       action_type,
       target_urn,
       retry_count,
       last_error,
       next_run_at,
       created_at,
       dead,
       failed_at
FROM sync_queue
WHERE ($1::text = 'all'
    OR ($1 = 'pending' AND dead = false AND retry_count = 0)
    OR ($1 = 'retrying' AND dead = false AND retry_count > 0)
    OR ($1 = 'dead' AND dead = true))
ORDER BY COALESCE(failed_at, next_run_at, created_at) DESC
LIMIT $2 OFFSET $3
