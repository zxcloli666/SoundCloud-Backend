WITH picked AS (
    SELECT queue.track_id,
           queue.attempts >= $2 AS exhausted
    FROM track_enrich_queue AS queue
    WHERE queue.enrich_state IN ('pending', 'failed')
      AND queue.next_run_at <= now()
      AND (
          queue.locked_at IS NULL
          OR queue.locked_at < now() - $1 * interval '1 second'
      )
      AND queue.index_priority BETWEEN $4 AND $5
    ORDER BY queue.index_priority, queue.next_run_at
    LIMIT $3
    FOR UPDATE SKIP LOCKED
), dying AS MATERIALIZED (
    SELECT track.id
    FROM tracks AS track
    JOIN picked
      ON picked.track_id = track.id
    WHERE picked.exhausted
    ORDER BY track.sc_track_id
    FOR NO KEY UPDATE OF track SKIP LOCKED
), retired AS (
    UPDATE tracks AS track
    SET enrich_state = 'dead',
        enriched_at = now()
    FROM dying
    WHERE track.id = dying.id
), expired AS (
    UPDATE track_enrich_queue AS queue
    SET locked_at = NULL,
        error = 'lease expired after final attempt'
    FROM dying
    WHERE queue.track_id = dying.id
)
UPDATE track_enrich_queue AS queue
SET locked_at = now(),
    attempts = queue.attempts + 1
FROM picked
JOIN tracks AS track
  ON track.id = picked.track_id
WHERE queue.track_id = picked.track_id
  AND NOT picked.exhausted
RETURNING track.id,
          track.sc_track_id,
          queue.attempts AS enrich_attempts
