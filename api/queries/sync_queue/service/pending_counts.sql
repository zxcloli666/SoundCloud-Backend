SELECT COUNT(*) FILTER (WHERE retry_count = 0 AND dead = false)::bigint AS "pending!",
       COUNT(*) FILTER (WHERE retry_count > 0 OR dead = true)::bigint AS "failed!",
       COUNT(*) FILTER (
           WHERE retry_count = 0 AND dead = false AND last_error IS NOT NULL AND next_run_at > now()
       )::bigint AS "delayed!",
       CEIL(EXTRACT(EPOCH FROM MIN(next_run_at - now()) FILTER (
           WHERE dead = false AND last_error IS NOT NULL AND next_run_at > now()
       )))::bigint AS retry_in_sec
FROM sync_queue
WHERE user_id = ANY ($1)
