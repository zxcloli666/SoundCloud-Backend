WITH picked AS (
    SELECT track.id,
           track.enrich_attempts >= $2 AS exhausted
    FROM tracks AS track
    WHERE track.enrich_state IN ('pending', 'failed')
      AND track.enrich_next_run_at <= now()
      AND (
          track.enrich_locked_at IS NULL
          OR track.enrich_locked_at < now() - $1 * interval '1 second'
      )
      AND track.index_priority BETWEEN $4 AND $5
    ORDER BY track.index_priority, track.enrich_next_run_at
    LIMIT $3
    FOR UPDATE SKIP LOCKED
), retired AS (
    UPDATE tracks AS track
    SET enrich_state = 'dead',
        enrich_locked_at = NULL,
        enrich_error = 'lease expired after final attempt',
        enriched_at = now()
    FROM picked
    WHERE track.id = picked.id
      AND picked.exhausted
)
UPDATE tracks AS track
SET enrich_locked_at = now(),
    enrich_attempts = track.enrich_attempts + 1
FROM picked
WHERE track.id = picked.id
  AND NOT picked.exhausted
RETURNING track.id,
          track.sc_track_id,
          track.enrich_attempts
