UPDATE track_highlight_dispatch AS dispatch
SET state = 'failed',
    reason = $3,
    updated_at = now()
FROM tracks AS track
WHERE track.sc_track_id = $1
  AND dispatch.track_id = track.id
  AND dispatch.attempt = $2
  AND dispatch.state = 'dispatched'
