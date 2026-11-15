WITH star AS MATERIALIZED (
    SELECT tiered.sc_track_id, tiered.tiered_at
    FROM star_track_tiers AS tiered
    WHERE tiered.tier = $4
      AND tiered.enrich_due
    ORDER BY tiered.tiered_at, tiered.sc_track_id
    LIMIT $5
), picked AS (
    SELECT queue.track_id,
           queue.attempts >= $2 AS exhausted
    FROM star
    JOIN tracks AS track
      ON track.sc_track_id = star.sc_track_id
    JOIN track_enrich_queue AS queue
      ON queue.track_id = track.id
    WHERE queue.enrich_state IN ('pending', 'failed')
      AND queue.next_run_at <= now()
      AND (
          queue.locked_at IS NULL
          OR queue.locked_at < now() - $1 * interval '1 second'
      )
    ORDER BY star.tiered_at, star.sc_track_id
    LIMIT $3
    FOR UPDATE OF queue SKIP LOCKED
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
