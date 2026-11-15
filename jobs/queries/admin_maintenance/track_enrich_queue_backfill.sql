WITH batch AS MATERIALIZED (
    SELECT track.id,
           track.enrich_state,
           track.index_priority,
           track.enrich_next_run_at,
           track.enrich_locked_at,
           track.enrich_attempts,
           track.enrich_error
    FROM tracks AS track
    WHERE track.id > $1
    ORDER BY track.id
    LIMIT $2
), written AS (
    INSERT INTO track_enrich_queue (
        track_id, enrich_state, index_priority, next_run_at, locked_at, attempts, error
    )
    SELECT batch.id,
           batch.enrich_state,
           batch.index_priority,
           batch.enrich_next_run_at,
           batch.enrich_locked_at,
           batch.enrich_attempts,
           batch.enrich_error
    FROM batch
    ORDER BY batch.id
    ON CONFLICT (track_id) DO NOTHING
    RETURNING track_enrich_queue.track_id
)
SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS "last?",
       (SELECT count(*) FROM batch) AS "scanned!",
       (SELECT count(*) FROM written) AS "inserted!"
