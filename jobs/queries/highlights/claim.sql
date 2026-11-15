WITH flight AS (
    SELECT count(*) AS busy
    FROM track_highlight_dispatch
    WHERE state = 'dispatched'
), due AS (
    SELECT dispatch.track_id
    FROM track_highlight_dispatch AS dispatch
    WHERE dispatch.state = 'queued'
      AND dispatch.next_run_at <= now()
    ORDER BY dispatch.popularity DESC, dispatch.track_id
    LIMIT GREATEST($1::bigint - (SELECT busy FROM flight), 0)
    FOR UPDATE SKIP LOCKED
)
UPDATE track_highlight_dispatch AS dispatch
SET state = 'dispatched',
    attempt = dispatch.attempt + CASE WHEN dispatch.reason = 'unanswered' THEN 0 ELSE 1 END,
    reason = NULL,
    dispatched_at = now(),
    updated_at = now()
FROM due, tracks AS track
WHERE dispatch.track_id = due.track_id
  AND track.id = due.track_id
RETURNING track.sc_track_id, dispatch.attempt, dispatch.version
