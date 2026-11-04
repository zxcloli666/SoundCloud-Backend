SELECT queue.next_run_at AS "waiting_since!"
FROM lyrics_lookup_queue AS queue
WHERE queue.next_run_at <= now()
  AND queue.claim_job_id IS NULL
ORDER BY queue.next_run_at, queue.created_at, queue.track_id
LIMIT 1
