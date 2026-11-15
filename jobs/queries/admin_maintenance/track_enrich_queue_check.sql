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
), bounds AS MATERIALIZED (
    SELECT last.id AS last,
           CASE
               WHEN (SELECT count(*) FROM batch) < $2 THEN 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid
               ELSE last.id
           END AS upper,
           (SELECT count(*) FROM batch) AS scanned
    FROM (SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS id) AS last
), stored AS MATERIALIZED (
    SELECT queue.track_id,
           queue.enrich_state,
           queue.index_priority,
           queue.next_run_at,
           queue.locked_at,
           queue.attempts,
           queue.error
    FROM bounds
    JOIN track_enrich_queue AS queue ON queue.track_id > $1 AND queue.track_id <= bounds.upper
)
SELECT bounds.last AS "last?",
       bounds.scanned AS "scanned!",
       ARRAY(
           SELECT coalesce(batch.id, stored.track_id)
           FROM batch
           FULL JOIN stored ON stored.track_id = batch.id
           WHERE batch.id IS NULL
              OR stored.track_id IS NULL
              OR (batch.enrich_state, batch.index_priority, batch.enrich_next_run_at, batch.enrich_locked_at,
                  batch.enrich_attempts, batch.enrich_error)
                 IS DISTINCT FROM
                 (stored.enrich_state, stored.index_priority, stored.next_run_at, stored.locked_at,
                  stored.attempts, stored.error)
           ORDER BY 1
       ) AS "mismatched!"
FROM bounds
