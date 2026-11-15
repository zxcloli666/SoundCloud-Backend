WITH locked AS MATERIALIZED (
    SELECT track.id, track.enrich_state
    FROM tracks AS track
    WHERE track.id = $1
    ORDER BY track.sc_track_id
    FOR NO KEY UPDATE OF track
), state AS (
    UPDATE tracks AS track
    SET enrich_state = 'failed'
    FROM locked
    WHERE track.id = locked.id
      AND locked.enrich_state <> 'failed'
)
UPDATE track_enrich_queue AS queue
SET locked_at = NULL,
    next_run_at = $2,
    error = $3
FROM locked
WHERE queue.track_id = locked.id
