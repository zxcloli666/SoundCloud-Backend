WITH locked AS MATERIALIZED (
    SELECT track.id
    FROM tracks AS track
    WHERE track.id = $1
    ORDER BY track.sc_track_id
    FOR NO KEY UPDATE OF track
), state AS (
    UPDATE tracks AS track
    SET enrich_state = 'dead',
        enriched_at = now()
    FROM locked
    WHERE track.id = locked.id
)
UPDATE track_enrich_queue AS queue
SET locked_at = NULL,
    error = $2
FROM locked
WHERE queue.track_id = locked.id
