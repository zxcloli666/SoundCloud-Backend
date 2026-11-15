UPDATE track_highlight_dispatch AS dispatch
SET state = 'queued',
    reason = 'unanswered',
    next_run_at = now() + interval '1 minute',
    updated_at = now()
FROM tracks AS track
WHERE track.sc_track_id = $1
  AND dispatch.track_id = track.id
  AND dispatch.state = 'dispatched'
  AND dispatch.attempt = $2
