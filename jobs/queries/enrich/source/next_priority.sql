SELECT track.enrich_next_run_at AS "waiting_since!"
FROM tracks AS track
WHERE track.enrich_state IN ('pending', 'failed')
  AND track.enrich_next_run_at <= now()
  AND (
      track.enrich_locked_at IS NULL
      OR track.enrich_locked_at < now() - $1 * interval '1 second'
  )
  AND track.enrich_attempts < $2
  AND track.index_priority BETWEEN $3 AND $4
ORDER BY track.index_priority, track.enrich_next_run_at
LIMIT 1
