UPDATE track_highlight_dispatch AS dispatch
SET state = 'queued',
    reason = 'unanswered',
    next_run_at = now(),
    updated_at = now()
WHERE dispatch.state = 'dispatched'
  AND dispatch.dispatched_at < now() - $1::bigint * interval '1 second'
