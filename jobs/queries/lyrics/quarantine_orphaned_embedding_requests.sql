WITH candidates AS MATERIALIZED (
    SELECT wire.sc_track_id
    FROM lyrics_embedding_wire_state AS wire
    WHERE wire.status = 'pending'
      AND CASE
          WHEN wire.result_consumer IS NULL
              THEN wire.first_publish_attempt_at
          ELSE wire.updated_at
      END < now() - $2::bigint * interval '1 second'
      AND (
          wire.result_lease_id IS NULL
          OR wire.result_lease_expires_at <= now()
      )
      AND NOT EXISTS (
          SELECT 1
          FROM lyrics_cache AS lyrics
          WHERE lyrics.sc_track_id = wire.sc_track_id
      )
    ORDER BY wire.first_publish_attempt_at, wire.sc_track_id
    FOR UPDATE OF wire SKIP LOCKED
    LIMIT $1
), quarantined AS (
    UPDATE lyrics_embedding_wire_state AS wire
    SET status = 'quarantined',
        completed_at = now(),
        quarantine_reason = 'lyrics_missing_after_result_timeout',
        result_lease_id = NULL,
        result_lease_expires_at = NULL,
        updated_at = now()
    FROM candidates
    WHERE wire.sc_track_id = candidates.sc_track_id
    RETURNING wire.sc_track_id
)
SELECT count(*)::bigint AS "quarantined!"
FROM quarantined
