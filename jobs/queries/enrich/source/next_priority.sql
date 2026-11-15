SELECT queue.next_run_at AS "waiting_since!"
FROM track_enrich_queue AS queue
WHERE queue.enrich_state IN ('pending', 'failed')
  AND queue.next_run_at <= now()
  AND (
      queue.locked_at IS NULL
      OR queue.locked_at < now() - $1 * interval '1 second'
  )
  AND queue.attempts < $2
  AND queue.index_priority BETWEEN $3 AND $4
ORDER BY queue.index_priority, queue.next_run_at
LIMIT 1
