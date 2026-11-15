WITH locked AS MATERIALIZED (
    SELECT track.id,
           track.enrich_state,
           track.index_priority,
           track.enrich_next_run_at,
           track.enrich_locked_at,
           track.enrich_attempts,
           track.enrich_error
    FROM tracks AS track
    WHERE track.id = ANY ($1)
    ORDER BY track.id
    FOR SHARE
), removed AS (
    DELETE FROM track_enrich_queue AS queue
    WHERE queue.track_id = ANY ($1)
      AND NOT EXISTS (SELECT 1 FROM locked WHERE locked.id = queue.track_id)
    RETURNING queue.track_id
), written AS (
    INSERT INTO track_enrich_queue AS stored (
        track_id, enrich_state, index_priority, next_run_at, locked_at, attempts, error
    )
    SELECT locked.id,
           locked.enrich_state,
           locked.index_priority,
           locked.enrich_next_run_at,
           locked.enrich_locked_at,
           locked.enrich_attempts,
           locked.enrich_error
    FROM locked
    ORDER BY locked.id
    ON CONFLICT (track_id) DO UPDATE
    SET enrich_state = EXCLUDED.enrich_state,
        index_priority = EXCLUDED.index_priority,
        next_run_at = EXCLUDED.next_run_at,
        locked_at = EXCLUDED.locked_at,
        attempts = EXCLUDED.attempts,
        error = EXCLUDED.error
    WHERE (stored.enrich_state, stored.index_priority, stored.next_run_at, stored.locked_at, stored.attempts,
           stored.error)
          IS DISTINCT FROM
          (EXCLUDED.enrich_state, EXCLUDED.index_priority, EXCLUDED.next_run_at, EXCLUDED.locked_at,
           EXCLUDED.attempts, EXCLUDED.error)
    RETURNING stored.track_id
)
SELECT (SELECT count(*) FROM removed) AS "removed!",
       (SELECT count(*) FROM written) AS "written!"
