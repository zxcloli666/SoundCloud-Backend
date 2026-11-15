UPDATE track_highlight_dispatch AS dispatch
SET state = CASE WHEN dispatch.failures + 1 >= $4 THEN 'failed' ELSE 'queued' END,
    failures = dispatch.failures + 1,
    reason = $3,
    next_run_at = now() + LEAST(interval '10 minutes' * power(2, dispatch.failures), interval '1 day'),
    updated_at = now()
FROM tracks AS track
WHERE track.sc_track_id = $1
  AND dispatch.track_id = track.id
  AND dispatch.attempt = $2
  AND dispatch.state = 'dispatched'
