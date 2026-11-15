WITH locked AS MATERIALIZED (
    SELECT track.id
    FROM tracks AS track
    WHERE track.enrich_state IN ('failed', 'dead')
    ORDER BY track.sc_track_id
    FOR NO KEY UPDATE OF track
), reset AS (
    UPDATE tracks
    SET enrich_state      = 'pending',
        enriched_at       = NULL,
        enrich_source     = NULL,
        enrich_confidence = NULL
    FROM locked
    WHERE tracks.id = locked.id
    RETURNING tracks.id
), queued AS (
    UPDATE track_enrich_queue AS queue
    SET attempts    = 0,
        next_run_at = now(),
        locked_at   = NULL,
        error       = NULL
    FROM reset
    WHERE queue.track_id = reset.id
)
SELECT count(*) AS "reset!"
FROM reset
