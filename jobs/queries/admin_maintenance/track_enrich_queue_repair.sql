WITH locked AS MATERIALIZED (
    SELECT track.id,
           track.enrich_state,
           track.index_priority
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
        track_id, enrich_state, index_priority, next_run_at
    )
    SELECT locked.id,
           locked.enrich_state,
           locked.index_priority,
           now()
    FROM locked
    ORDER BY locked.id
    ON CONFLICT (track_id) DO UPDATE
    SET enrich_state = EXCLUDED.enrich_state,
        index_priority = EXCLUDED.index_priority
    WHERE (stored.enrich_state, stored.index_priority)
          IS DISTINCT FROM
          (EXCLUDED.enrich_state, EXCLUDED.index_priority)
    RETURNING stored.track_id
)
SELECT (SELECT count(*) FROM removed) AS "removed!",
       (SELECT count(*) FROM written) AS "written!"
